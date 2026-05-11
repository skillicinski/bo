// bo query — LLM-synthesized answers with citations
//
// Pipeline: extract terms → retrieve leaves → assemble context → synthesize → format
//
// This module is self-contained and does not share retrieval logic with
// `cli::search` (different semantics: OR vs AND, different purpose).

use crate::domain::frontmatter;
use crate::domain::index;
use crate::engine::llm::{
    complete_with_policy, context_window_tokens, FinishReason, LlmCallPolicy, LlmError,
    LlmProvider, Message, OpenAiProvider,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::Path;
use std::time::Duration;

// ── constants ────────────────────────────────────────────────────────────────

const RETRIEVAL_TOP_K: usize = 10;
const DEPTH_TOP_K: usize = 5;
const QUERY_MAX_COMPLETION_TOKENS: u32 = 2048;
const QUERY_PROMPT_OVERHEAD_TOKENS: usize = 4096;
const MIN_QUERY_SOURCE_WORDS: usize = 1000;
const TOKENS_TO_WORDS_NUMERATOR: usize = 3;
const TOKENS_TO_WORDS_DENOMINATOR: usize = 4;
const SUMMARY_FALLBACK_WORDS: usize = 200;

const QUERY_LLM_POLICY: LlmCallPolicy = LlmCallPolicy {
    timeout: Duration::from_secs(60),
    max_attempts: 3,
    initial_backoff: Duration::from_secs(1),
};

const STOP_WORDS: &[&str] = &[
    "what", "which", "who", "whom", "where", "when", "why", "how", "is", "are", "was", "were",
    "am", "do", "does", "did", "has", "have", "had", "can", "could", "would", "should", "will",
    "shall", "the", "a", "an", "of", "in", "on", "at", "to", "for", "with", "by", "from", "about",
    "between", "and", "or", "but", "not", "no", "if", "then", "than", "that", "this", "these",
    "those", "it", "its", "be", "been", "being", "my", "your", "our", "their", "me", "you", "us",
    "them", "he", "she", "we", "they", "his", "her",
];

// ── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub answer: String,
    pub citations: Vec<Citation>,
    pub model: String,
    pub leaves_consulted: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct Citation {
    pub slug: String,
    pub title: String,
    pub file: String,
}

#[derive(Debug)]
pub enum QueryError {
    /// No API key / provider configured
    NoProvider(String),
    /// Could not extract meaningful terms from question
    NoTerms,
    /// No relevant sources found in tree
    NoResults,
    /// Tree has no leaves
    EmptyTree,
    /// Index read or file I/O error
    Io(String),
    /// Configured query model has no known context window
    UnknownModelContext { model: String },
    /// Known model has too little context after reserved prompt/completion budget
    ContextBudgetExhausted {
        model: String,
        context_tokens: usize,
        reserved_tokens: usize,
    },
    /// LLM output hit the completion token limit
    Truncated,
    /// LLM output was blocked by content filtering
    ContentFilter,
    /// LLM call failed
    Llm(LlmError),
    /// LLM response could not be parsed
    Parse(String),
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryError::NoProvider(msg) => write!(f, "{}", msg),
            QueryError::NoTerms => write!(
                f,
                "could not extract meaningful terms from question — try rephrasing with specific keywords"
            ),
            QueryError::NoResults => write!(f, "no relevant sources found in tree"),
            QueryError::EmptyTree => write!(f, "no sources collected yet"),
            QueryError::Io(msg) => write!(f, "{}", msg),
            QueryError::UnknownModelContext { model } => write!(
                f,
                "unknown context window for query_model '{}' — choose a known model or add its context window",
                model
            ),
            QueryError::ContextBudgetExhausted {
                model,
                context_tokens,
                reserved_tokens,
            } => write!(
                f,
                "query exhausted model context for '{}' — context window is {} tokens and {} tokens are reserved before source context",
                model, context_tokens, reserved_tokens
            ),
            QueryError::Truncated => write!(
                f,
                "query synthesis was truncated — try a model with larger output capacity"
            ),
            QueryError::ContentFilter => write!(f, "query synthesis was blocked by content filter"),
            QueryError::Llm(e) => write!(f, "{}", e),
            QueryError::Parse(msg) => write!(f, "synthesis failed — {}", msg),
        }
    }
}

impl QueryError {
    /// Exit code per spec: 1 = no results, 2 = provider/config/system error
    pub fn exit_code(&self) -> i32 {
        match self {
            QueryError::NoResults | QueryError::EmptyTree => 1,
            _ => 2,
        }
    }
}

// ── internal types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct QueryContextBudget {
    pub model: String,
    pub context_tokens: usize,
    pub reserved_tokens: usize,
    pub source_tokens: usize,
    pub source_words: usize,
}

#[derive(Debug, Clone)]
struct RetrievedLeaf {
    slug: String,
    title: String,
    url: String,
    file: String,
    summary: String,
    body: String,
    score: f64,
}

// ── term extraction ──────────────────────────────────────────────────────────

/// Extract meaningful search terms from a natural-language question.
/// Strips stop words, possessives, boundary punctuation, and terms < 2 chars.
pub fn extract_terms(question: &str) -> Result<Vec<String>, QueryError> {
    let terms: Vec<String> = question
        .split_whitespace()
        .map(strip_punctuation)
        .map(|w| strip_possessive(&w))
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 2)
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .collect();

    if terms.is_empty() {
        return Err(QueryError::NoTerms);
    }
    Ok(terms)
}

/// Strip leading/trailing punctuation from a word.
fn strip_punctuation(word: &str) -> String {
    word.trim_matches(|c: char| c.is_ascii_punctuation())
        .to_string()
}

/// Strip common possessive/contraction suffixes: 's, 't, 're, 've, 'd, 'll
fn strip_possessive(word: &str) -> String {
    for suffix in &[
        "'s",
        "'t",
        "'re",
        "'ve",
        "'d",
        "'ll",
        "\u{2019}s",
        "\u{2019}t",
    ] {
        if let Some(stem) = word.strip_suffix(suffix) {
            if !stem.is_empty() {
                return stem.to_string();
            }
        }
    }
    word.to_string()
}

// ── retrieval ────────────────────────────────────────────────────────────────

/// Retrieve top-k leaves scored by term density (OR semantics).
fn retrieve_leaves(tree_dir: &Path, terms: &[String]) -> Result<Vec<RetrievedLeaf>, QueryError> {
    let index_path = tree_dir.join("index.jsonl");
    let entries =
        index::read_index(&index_path).map_err(|e| QueryError::Io(format!("index: {}", e)))?;

    if entries.is_empty() {
        return Err(QueryError::EmptyTree);
    }

    let mut scored: Vec<RetrievedLeaf> = Vec::new();

    for entry in &entries {
        let path = tree_dir.join(&entry.file);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue, // skip unreadable leaves
        };

        let (mapping, body) = match frontmatter::parse(&content) {
            Ok(v) => v,
            Err(_) => continue, // skip malformed leaves
        };

        let title = extract_yaml_string(&mapping, "title").unwrap_or_else(|| entry.title.clone());
        let url = extract_yaml_string(&mapping, "url").unwrap_or_else(|| entry.url.clone());
        let summary =
            extract_yaml_string(&mapping, "summary").unwrap_or_else(|| summary_fallback(&body));

        let slug = slug_from_file(&entry.file);

        // Score: OR semantics — count occurrences of each term, normalize by word count
        let searchable = format!("{} {} {}", title, summary, body).to_lowercase();
        let word_count = searchable.split_whitespace().count();
        if word_count == 0 {
            continue;
        }

        let total_hits: usize = terms
            .iter()
            .map(|term| searchable.matches(term.as_str()).count())
            .sum();

        if total_hits == 0 {
            continue;
        }

        let score = (total_hits as f64 * 1000.0) / word_count as f64;

        scored.push(RetrievedLeaf {
            slug,
            title,
            url,
            file: entry.file.clone(),
            summary,
            body,
            score,
        });
    }

    if scored.is_empty() {
        return Err(QueryError::NoResults);
    }

    // Sort by score descending
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(RETRIEVAL_TOP_K);

    Ok(scored)
}

// ── context assembly ─────────────────────────────────────────────────────────

fn compute_query_context_budget(model: &str) -> Result<QueryContextBudget, QueryError> {
    let Some(context_tokens) = context_window_tokens(model) else {
        return Err(QueryError::UnknownModelContext {
            model: model.to_string(),
        });
    };

    compute_query_context_budget_from_tokens(model, context_tokens)
}

fn compute_query_context_budget_from_tokens(
    model: &str,
    context_tokens: usize,
) -> Result<QueryContextBudget, QueryError> {
    let reserved_tokens = QUERY_PROMPT_OVERHEAD_TOKENS + QUERY_MAX_COMPLETION_TOKENS as usize;
    if context_tokens <= reserved_tokens {
        return Err(QueryError::ContextBudgetExhausted {
            model: model.to_string(),
            context_tokens,
            reserved_tokens,
        });
    }

    let source_tokens = context_tokens - reserved_tokens;
    let source_words = (source_tokens * TOKENS_TO_WORDS_NUMERATOR) / TOKENS_TO_WORDS_DENOMINATOR;

    if source_words < MIN_QUERY_SOURCE_WORDS {
        return Err(QueryError::ContextBudgetExhausted {
            model: model.to_string(),
            context_tokens,
            reserved_tokens,
        });
    }

    Ok(QueryContextBudget {
        model: model.to_string(),
        context_tokens,
        reserved_tokens,
        source_tokens,
        source_words,
    })
}

/// Assemble LLM context from retrieved leaves.
/// Returns (context_string, leaves_consulted_count).
fn assemble_context(leaves: &[RetrievedLeaf], source_word_budget: usize) -> (String, usize) {
    let mut context = String::new();
    let mut word_budget = source_word_budget;
    let mut consulted = 0;

    // Breadth tier: all retrieved leaves get summary context
    context.push_str("## Available sources\n\n");
    for leaf in leaves {
        let entry = format!(
            "- [[{}]] — {} ({})\n  Summary: {}\n\n",
            leaf.slug, leaf.title, leaf.url, leaf.summary
        );
        let words = entry.split_whitespace().count();
        if words > word_budget {
            break;
        }
        context.push_str(&entry);
        word_budget = word_budget.saturating_sub(words);
    }

    // Depth tier: top-k get full body
    let depth_count = leaves.len().min(DEPTH_TOP_K);
    if depth_count > 0 {
        context.push_str("## Full source content\n\n");
    }
    for leaf in leaves.iter().take(depth_count) {
        let body_words: Vec<&str> = leaf.body.split_whitespace().collect();
        let usable_words = body_words.len().min(word_budget);
        if usable_words == 0 {
            break;
        }
        let truncated_body: String = body_words[..usable_words].join(" ");

        let entry = format!(
            "### [[{}]] — {}\n\n{}\n\n",
            leaf.slug, leaf.title, truncated_body
        );
        let entry_words = entry.split_whitespace().count();
        context.push_str(&entry);
        word_budget = word_budget.saturating_sub(entry_words);
        consulted += 1;
    }

    (context, consulted)
}

// ── synthesis ────────────────────────────────────────────────────────────────

const SYNTHESIS_SYSTEM_PROMPT: &str = "\
You are a knowledge base assistant. Answer the user's question using ONLY the \
provided source material. Follow these rules strictly:

1. Cite sources using [[slug]] wikilink format inline in your prose.
2. If the sources don't contain enough information to answer, say so explicitly.
3. Do not invent information not present in the sources.
4. Keep your answer concise — 1 to 3 paragraphs.
5. The cited_slugs array must contain every slug you reference in your answer.";

#[derive(Deserialize)]
struct SynthesisResponse {
    answer: String,
    cited_slugs: Vec<String>,
}

/// Run synthesis with an injectable provider.
fn synthesize_with_provider(
    question: &str,
    context: &str,
    provider: &dyn LlmProvider,
    model: &str,
    policy: LlmCallPolicy,
) -> Result<SynthesisResponse, QueryError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| QueryError::Io(format!("failed to create async runtime: {}", e)))?;

    let user_message = format!(
        "<question>{}</question>\n\n<sources>\n{}</sources>",
        question, context
    );

    let messages = vec![
        Message::system(SYNTHESIS_SYSTEM_PROMPT),
        Message::user(user_message),
    ];

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "answer": {
                "type": "string",
                "description": "Prose answer with [[slug]] citations inline"
            },
            "cited_slugs": {
                "type": "array",
                "items": { "type": "string" },
                "description": "List of leaf slugs actually cited in the answer"
            }
        },
        "required": ["answer", "cited_slugs"],
        "additionalProperties": false
    });

    let response = rt
        .block_on(complete_with_policy(
            provider,
            &messages,
            model,
            QUERY_MAX_COMPLETION_TOKENS,
            Some(&schema),
            policy,
        ))
        .map_err(QueryError::Llm)?;

    match response.finish_reason {
        FinishReason::Stop => {}
        FinishReason::Length => return Err(QueryError::Truncated),
        FinishReason::ContentFilter => return Err(QueryError::ContentFilter),
        FinishReason::Other(reason) => {
            return Err(QueryError::Llm(LlmError::Api(format!(
                "unexpected finish reason: {}",
                reason
            ))));
        }
    }

    let parsed: SynthesisResponse = serde_json::from_str(&response.content)
        .map_err(|e| QueryError::Parse(format!("invalid response from model: {}", e)))?;

    Ok(parsed)
}

// ── citation validation ──────────────────────────────────────────────────────

/// Validate citations against the retrieval set.
/// Strips invalid slugs from cited_slugs and removes invalid [[slug]] from prose.
fn validate_citations(
    response: SynthesisResponse,
    retrieved: &[RetrievedLeaf],
) -> (String, Vec<Citation>) {
    let valid_slugs: HashSet<&str> = retrieved.iter().map(|l| l.slug.as_str()).collect();

    let (answer, prose_slugs) =
        sanitize_wikilinks_and_collect_valid(&response.answer, &valid_slugs);

    let mut ordered_slugs: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for slug in prose_slugs
        .into_iter()
        .chain(response.cited_slugs.into_iter())
    {
        if valid_slugs.contains(slug.as_str()) && seen.insert(slug.clone()) {
            ordered_slugs.push(slug);
        }
    }

    let citations: Vec<Citation> = ordered_slugs
        .iter()
        .filter_map(|slug| {
            retrieved
                .iter()
                .find(|l| l.slug == *slug)
                .map(|l| Citation {
                    slug: l.slug.clone(),
                    title: l.title.clone(),
                    file: l.file.clone(),
                })
        })
        .collect();

    (answer, citations)
}

fn sanitize_wikilinks_and_collect_valid(
    answer: &str,
    valid_slugs: &HashSet<&str>,
) -> (String, Vec<String>) {
    let mut sanitized = String::with_capacity(answer.len());
    let mut valid_in_prose = Vec::new();
    let mut i = 0;

    while i < answer.len() {
        let rest = &answer[i..];
        if !rest.starts_with("[[") {
            let ch = rest.chars().next().expect("non-empty slice");
            sanitized.push(ch);
            i += ch.len_utf8();
            continue;
        }

        let Some(relative_end) = rest[2..].find("]]") else {
            sanitized.push_str(rest);
            break;
        };
        let inner_start = i + 2;
        let inner_end = inner_start + relative_end;
        let span_end = inner_end + 2;
        let inner = &answer[inner_start..inner_end];
        let span = &answer[i..span_end];

        if inner.is_empty() || inner.contains('[') || inner.contains(']') {
            sanitized.push_str(span);
        } else if valid_slugs.contains(inner) {
            sanitized.push_str(span);
            valid_in_prose.push(inner.to_string());
        } else {
            sanitized.push_str(inner);
        }

        i = span_end;
    }

    (sanitized, valid_in_prose)
}

// ── output formatting ────────────────────────────────────────────────────────

/// Render human-readable output.
pub fn render_human(result: &QueryResult) -> String {
    let mut output = result.answer.clone();
    if !result.citations.is_empty() {
        output.push_str("\n\nSources:\n");
        for c in &result.citations {
            output.push_str(&format!("  • [[{}]] — {}\n", c.slug, c.title));
        }
    }
    output
}

/// Render JSON output (ADR-002 compliant).
pub fn render_json(result: &QueryResult) -> Result<String, QueryError> {
    serde_json::to_string_pretty(result)
        .map_err(|e| QueryError::Parse(format!("JSON serialization failed: {}", e)))
}

// ── orchestrator ─────────────────────────────────────────────────────────────

/// Run the full query pipeline.
pub fn run(
    tree_dir: &Path,
    question: &str,
    api_key: &str,
    model: &str,
) -> Result<QueryResult, QueryError> {
    let provider = OpenAiProvider::new(api_key);
    run_with_provider(tree_dir, question, &provider, model)
}

/// Run the full query pipeline with an injectable provider (for testing).
pub fn run_with_provider(
    tree_dir: &Path,
    question: &str,
    provider: &dyn LlmProvider,
    model: &str,
) -> Result<QueryResult, QueryError> {
    run_with_provider_and_policy(tree_dir, question, provider, model, QUERY_LLM_POLICY)
}

fn run_with_provider_and_policy(
    tree_dir: &Path,
    question: &str,
    provider: &dyn LlmProvider,
    model: &str,
    policy: LlmCallPolicy,
) -> Result<QueryResult, QueryError> {
    let terms = extract_terms(question)?;
    let budget = compute_query_context_budget(model)?;

    eprintln!("searching...");
    let retrieved = retrieve_leaves(tree_dir, &terms)?;

    let (context, consulted) = assemble_context(&retrieved, budget.source_words);

    eprintln!("synthesizing...");
    let response = synthesize_with_provider(question, &context, provider, model, policy)?;

    let (answer, citations) = validate_citations(response, &retrieved);

    Ok(QueryResult {
        answer,
        citations,
        model: model.to_string(),
        leaves_consulted: consulted,
    })
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Extract slug from file path: "leaves/foo-bar.md" → "foo-bar"
fn slug_from_file(file: &str) -> String {
    Path::new(file)
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file.to_string())
}

/// Extract a string value from a YAML mapping by key.
fn extract_yaml_string(mapping: &serde_yaml_ng::Mapping, key: &str) -> Option<String> {
    mapping
        .get(serde_yaml_ng::Value::String(key.to_string()))
        .and_then(|v| match v {
            serde_yaml_ng::Value::String(s) => Some(s.clone()),
            _ => None,
        })
}

/// Generate a summary fallback from the first ~200 words of body.
fn summary_fallback(body: &str) -> String {
    let words: Vec<&str> = body.split_whitespace().collect();
    if words.len() <= SUMMARY_FALLBACK_WORDS {
        words.join(" ")
    } else {
        words[..SUMMARY_FALLBACK_WORDS].join(" ")
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "../tests/cli_query_tests.rs"]
mod tests;
