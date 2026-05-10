// bo query — LLM-synthesized answers with citations
//
// Pipeline: extract terms → retrieve leaves → assemble context → synthesize → format
//
// This module is self-contained and does not share retrieval logic with
// `cli::search` (different semantics: OR vs AND, different purpose).

use crate::domain::frontmatter;
use crate::domain::index;
use crate::engine::llm::{LlmError, LlmProvider, Message, OpenAiProvider};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::Path;

// ── constants ────────────────────────────────────────────────────────────────

const RETRIEVAL_TOP_K: usize = 10;
const DEPTH_TOP_K: usize = 5;
const TOKEN_BUDGET_WORDS: usize = 60_000;
const SUMMARY_FALLBACK_WORDS: usize = 200;

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

/// Assemble LLM context from retrieved leaves.
/// Returns (context_string, leaves_consulted_count).
fn assemble_context(leaves: &[RetrievedLeaf]) -> (String, usize) {
    let mut context = String::new();
    let mut word_budget = TOKEN_BUDGET_WORDS;
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

/// Run the synthesis LLM call. Returns raw SynthesisResponse.
fn synthesize(
    question: &str,
    context: &str,
    api_key: &str,
    model: &str,
) -> Result<SynthesisResponse, QueryError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| QueryError::Io(format!("failed to create async runtime: {}", e)))?;

    let provider = OpenAiProvider::new(api_key);

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
        .block_on(provider.complete(&messages, model, 2048, Some(&schema)))
        .map_err(QueryError::Llm)?;

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
    let valid_slugs: std::collections::HashSet<&str> =
        retrieved.iter().map(|l| l.slug.as_str()).collect();

    // Filter cited_slugs to only valid ones
    let validated_slugs: Vec<String> = response
        .cited_slugs
        .into_iter()
        .filter(|s| valid_slugs.contains(s.as_str()))
        .collect();

    // Remove invalid [[slug]] wikilinks from answer prose
    let wikilink_re = Regex::new(r"\[\[([^\]]+)\]\]").unwrap();
    let answer = wikilink_re
        .replace_all(&response.answer, |caps: &regex::Captures| {
            let slug = &caps[1];
            if valid_slugs.contains(slug) {
                format!("[[{}]]", slug)
            } else {
                slug.to_string()
            }
        })
        .to_string();

    // Build citation metadata
    let citations: Vec<Citation> = validated_slugs
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
    let terms = extract_terms(question)?;

    eprintln!("searching...");
    let retrieved = retrieve_leaves(tree_dir, &terms)?;

    let (context, consulted) = assemble_context(&retrieved);

    eprintln!("synthesizing...");
    let response = synthesize(question, &context, api_key, model)?;

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
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── term extraction tests ────────────────────────────────────────────

    #[test]
    fn extract_basic_question() {
        let terms = extract_terms("what are the tradeoffs of Rust's ownership model?").unwrap();
        assert_eq!(terms, vec!["tradeoffs", "rust", "ownership", "model"]);
    }

    #[test]
    fn extract_single_word() {
        let terms = extract_terms("ownership").unwrap();
        assert_eq!(terms, vec!["ownership"]);
    }

    #[test]
    fn extract_all_stop_words_returns_error() {
        let err = extract_terms("what is it?").unwrap_err();
        assert!(matches!(err, QueryError::NoTerms));
    }

    #[test]
    fn extract_strips_possessives() {
        let terms = extract_terms("Rust's borrow checker").unwrap();
        assert_eq!(terms, vec!["rust", "borrow", "checker"]);
    }

    #[test]
    fn extract_drops_short_terms() {
        // "a" and "I" are < 2 chars and should be dropped
        let terms = extract_terms("a big I see").unwrap();
        assert_eq!(terms, vec!["big", "see"]);
    }

    #[test]
    fn extract_strips_boundary_punctuation() {
        let terms = extract_terms("(memory) safety! \"lifetimes\"").unwrap();
        assert_eq!(terms, vec!["memory", "safety", "lifetimes"]);
    }

    #[test]
    fn extract_unicode_possessive() {
        // Smart quote possessive: Rust\u{2019}s
        let terms = extract_terms("Rust\u{2019}s ownership").unwrap();
        assert_eq!(terms, vec!["rust", "ownership"]);
    }

    // ── retrieval tests ──────────────────────────────────────────────────

    fn make_leaf(
        dir: &Path,
        filename: &str,
        title: &str,
        url: &str,
        summary: Option<&str>,
        body: &str,
    ) {
        let leaves_dir = dir.join("leaves");
        fs::create_dir_all(&leaves_dir).unwrap();

        let mut content = String::from("---\n");
        content.push_str(&format!("title: \"{}\"\n", title));
        content.push_str(&format!("url: \"{}\"\n", url));
        if let Some(s) = summary {
            content.push_str(&format!("summary: \"{}\"\n", s));
        }
        content.push_str("---\n\n");
        content.push_str(body);

        fs::write(leaves_dir.join(filename), content).unwrap();
    }

    fn make_index(dir: &Path, entries: &[(&str, &str, &str)]) {
        let mut lines = String::new();
        for (file, title, url) in entries {
            lines.push_str(&format!(
                "{{\"file\":\"{}\",\"title\":\"{}\",\"url\":\"{}\"}}\n",
                file, title, url
            ));
        }
        fs::write(dir.join("index.jsonl"), lines).unwrap();
    }

    #[test]
    fn retrieve_or_semantics_scores_partial_matches() {
        let dir = TempDir::new().unwrap();
        let tree = dir.path();

        make_leaf(tree, "ownership.md", "Understanding Ownership", "https://example.com/ownership", Some("Rust ownership and borrowing"), "Ownership is a key feature of Rust. It ensures memory safety without a garbage collector.");
        make_leaf(
            tree,
            "lifetimes.md",
            "Lifetimes in Rust",
            "https://example.com/lifetimes",
            Some("How lifetimes work"),
            "Lifetimes ensure references are valid. They are part of Rust's type system.",
        );
        make_leaf(
            tree,
            "cooking.md",
            "Cooking Tips",
            "https://example.com/cooking",
            Some("How to cook pasta"),
            "Boil water and add salt. Cook pasta for 10 minutes.",
        );

        make_index(
            tree,
            &[
                (
                    "leaves/ownership.md",
                    "Understanding Ownership",
                    "https://example.com/ownership",
                ),
                (
                    "leaves/lifetimes.md",
                    "Lifetimes in Rust",
                    "https://example.com/lifetimes",
                ),
                (
                    "leaves/cooking.md",
                    "Cooking Tips",
                    "https://example.com/cooking",
                ),
            ],
        );

        let terms = vec!["rust".to_string(), "ownership".to_string()];
        let results = retrieve_leaves(tree, &terms).unwrap();

        // ownership leaf should rank highest (both terms match densely)
        assert_eq!(results[0].slug, "ownership");
        // lifetimes should match (contains "rust")
        assert!(results.iter().any(|r| r.slug == "lifetimes"));
        // cooking should NOT match
        assert!(!results.iter().any(|r| r.slug == "cooking"));
    }

    #[test]
    fn retrieve_empty_tree_returns_error() {
        let dir = TempDir::new().unwrap();
        let tree = dir.path();
        make_index(tree, &[]);

        let err = retrieve_leaves(tree, &["rust".to_string()]).unwrap_err();
        assert!(matches!(err, QueryError::EmptyTree));
    }

    #[test]
    fn retrieve_no_matches_returns_error() {
        let dir = TempDir::new().unwrap();
        let tree = dir.path();

        make_leaf(
            tree,
            "cooking.md",
            "Cooking Tips",
            "https://example.com/cooking",
            Some("How to cook"),
            "Boil water.",
        );
        make_index(
            tree,
            &[(
                "leaves/cooking.md",
                "Cooking Tips",
                "https://example.com/cooking",
            )],
        );

        let err = retrieve_leaves(tree, &["rust".to_string()]).unwrap_err();
        assert!(matches!(err, QueryError::NoResults));
    }

    #[test]
    fn retrieve_missing_summary_uses_body_fallback() {
        let dir = TempDir::new().unwrap();
        let tree = dir.path();

        // No summary field in frontmatter
        make_leaf(
            tree,
            "nosummary.md",
            "No Summary Leaf",
            "https://example.com/ns",
            None,
            "This leaf has no summary field but has a body about Rust programming.",
        );
        make_index(
            tree,
            &[(
                "leaves/nosummary.md",
                "No Summary Leaf",
                "https://example.com/ns",
            )],
        );

        let terms = vec!["rust".to_string()];
        let results = retrieve_leaves(tree, &terms).unwrap();

        assert_eq!(results[0].slug, "nosummary");
        // Summary should be the body fallback (body is short, so full body used)
        assert!(results[0].summary.contains("Rust programming"));
    }

    // ── helper tests ─────────────────────────────────────────────────────

    #[test]
    fn slug_from_file_strips_dir_and_extension() {
        assert_eq!(slug_from_file("leaves/foo-bar.md"), "foo-bar");
        assert_eq!(slug_from_file("leaves/sub/deep.md"), "deep");
        assert_eq!(slug_from_file("simple.md"), "simple");
    }

    // ── citation validation tests ────────────────────────────────────────

    #[test]
    fn validate_strips_invalid_citations() {
        let retrieved = vec![RetrievedLeaf {
            slug: "valid-leaf".to_string(),
            title: "Valid Leaf".to_string(),
            url: "https://example.com".to_string(),
            file: "leaves/valid-leaf.md".to_string(),
            summary: "summary".to_string(),
            body: "body".to_string(),
            score: 1.0,
        }];

        let response = SynthesisResponse {
            answer: "Answer cites [[valid-leaf]] and [[hallucinated]] sources.".to_string(),
            cited_slugs: vec!["valid-leaf".to_string(), "hallucinated".to_string()],
        };

        let (answer, citations) = validate_citations(response, &retrieved);

        // Invalid slug removed from prose
        assert!(answer.contains("[[valid-leaf]]"));
        assert!(!answer.contains("[[hallucinated]]"));
        assert!(answer.contains("hallucinated")); // text preserved, brackets removed

        // Invalid slug removed from citations list
        assert_eq!(citations.len(), 1);
        assert_eq!(citations[0].slug, "valid-leaf");
    }

    #[test]
    fn validate_preserves_all_valid_citations() {
        let retrieved = vec![
            RetrievedLeaf {
                slug: "leaf-a".to_string(),
                title: "Leaf A".to_string(),
                url: "https://a.com".to_string(),
                file: "leaves/leaf-a.md".to_string(),
                summary: "s".to_string(),
                body: "b".to_string(),
                score: 1.0,
            },
            RetrievedLeaf {
                slug: "leaf-b".to_string(),
                title: "Leaf B".to_string(),
                url: "https://b.com".to_string(),
                file: "leaves/leaf-b.md".to_string(),
                summary: "s".to_string(),
                body: "b".to_string(),
                score: 0.5,
            },
        ];

        let response = SynthesisResponse {
            answer: "See [[leaf-a]] and [[leaf-b]] for details.".to_string(),
            cited_slugs: vec!["leaf-a".to_string(), "leaf-b".to_string()],
        };

        let (answer, citations) = validate_citations(response, &retrieved);

        assert!(answer.contains("[[leaf-a]]"));
        assert!(answer.contains("[[leaf-b]]"));
        assert_eq!(citations.len(), 2);
    }

    // ── context assembly tests ───────────────────────────────────────────

    #[test]
    fn assemble_respects_depth_limit() {
        let leaves: Vec<RetrievedLeaf> = (0..10)
            .map(|i| RetrievedLeaf {
                slug: format!("leaf-{}", i),
                title: format!("Leaf {}", i),
                url: format!("https://example.com/{}", i),
                file: format!("leaves/leaf-{}.md", i),
                summary: "Short summary.".to_string(),
                body: "Some body content here.".to_string(),
                score: 10.0 - i as f64,
            })
            .collect();

        let (context, consulted) = assemble_context(&leaves);

        // All 10 appear in breadth tier
        for i in 0..10 {
            assert!(context.contains(&format!("[[leaf-{}]]", i)));
        }
        // Only top 5 get full body
        assert_eq!(consulted, 5);
        assert!(context.contains("### [[leaf-0]]"));
        assert!(context.contains("### [[leaf-4]]"));
        assert!(!context.contains("### [[leaf-5]]"));
    }

    #[test]
    fn assemble_truncates_on_word_budget() {
        // Create a leaf with a massive body
        let big_body = "word ".repeat(TOKEN_BUDGET_WORDS + 1000);
        let leaves = vec![RetrievedLeaf {
            slug: "big".to_string(),
            title: "Big Leaf".to_string(),
            url: "https://example.com/big".to_string(),
            file: "leaves/big.md".to_string(),
            summary: "Summary.".to_string(),
            body: big_body,
            score: 10.0,
        }];

        let (context, consulted) = assemble_context(&leaves);

        // Should not exceed budget significantly
        let word_count = context.split_whitespace().count();
        assert!(word_count <= TOKEN_BUDGET_WORDS + 100); // small overhead from formatting
        assert_eq!(consulted, 1);
    }
}
