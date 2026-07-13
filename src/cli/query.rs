// bo query — LLM-synthesized answers with citations
//
// Pipeline: extract terms → retrieve leaves → assemble context → synthesize → format
//
// The capability layers (term extraction, retrieval, relevance validation,
// context assembly + budget, citation validation) live in `engine::retrieval`.
// This module is the command's thin composition: argv → primitives → synthesize
// → render. Nothing below the entry point prints.

use crate::cli::json::JsonError;
use crate::engine::llm::{
    complete_with_policy, FinishReason, LlmCallPolicy, LlmError, LlmProvider, Message, Model,
};
use crate::engine::retrieval::{
    assemble_context, compute_context_budget, extract_terms, retrieve_docs, validate_citations,
    validate_relevance, RetrievalError, RetrievedDoc, SynthesisResponse, MAX_COMPLETION_TOKENS,
};
use serde::Serialize;
use serde_json::json;
use std::fmt;
use std::path::Path;
use std::time::Duration;

// Re-export the capability types this command surfaces, so `query::Citation`
// and `query::LowRelevanceReason` remain stable addresses for callers/tests.
pub use crate::engine::retrieval::{Citation, LowRelevanceReason};

pub const QUERY_LLM_POLICY: LlmCallPolicy = LlmCallPolicy {
    timeout: Duration::from_secs(60),
    max_attempts: 3,
    initial_backoff: Duration::from_secs(1),
};

// ── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct QueryResult {
    pub answer: String,
    pub citations: Vec<Citation>,
    pub model: String,
    pub leaves_consulted: usize,
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
    /// Retrieved matches are too weak or generic to support synthesis
    LowRelevance {
        reason: LowRelevanceReason,
        matched_sources: usize,
    },
    /// Synthesis produced zero valid citations — tree doesn't cover the question
    InsufficientSources { leaves_consulted: usize },
}

impl From<RetrievalError> for QueryError {
    fn from(error: RetrievalError) -> Self {
        match error {
            RetrievalError::NoTerms => QueryError::NoTerms,
            RetrievalError::NoResults => QueryError::NoResults,
            RetrievalError::EmptyTree => QueryError::EmptyTree,
            RetrievalError::Io(msg) => QueryError::Io(msg),
            RetrievalError::ContextBudgetExhausted {
                model,
                context_tokens,
                reserved_tokens,
            } => QueryError::ContextBudgetExhausted {
                model,
                context_tokens,
                reserved_tokens,
            },
            RetrievalError::LowRelevance {
                reason,
                matched_sources,
            } => QueryError::LowRelevance {
                reason,
                matched_sources,
            },
        }
    }
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
            QueryError::LowRelevance { reason, .. } => match reason {
                LowRelevanceReason::WeakMatches => write!(
                    f,
                    "found matching sources, but they were not relevant enough to answer"
                ),
                LowRelevanceReason::GenericTerms => write!(
                    f,
                    "query terms were too generic to identify relevant sources"
                ),
            },
            QueryError::InsufficientSources { leaves_consulted } => write!(
                f,
                "searched {} sources but could not produce a grounded answer",
                leaves_consulted
            ),
        }
    }
}

impl QueryError {
    /// Exit code per spec: 1 = no-answer, 2 = provider/config/system error.
    pub fn exit_code(&self) -> i32 {
        match self {
            QueryError::NoResults
            | QueryError::EmptyTree
            | QueryError::LowRelevance { .. }
            | QueryError::InsufficientSources { .. } => 1,
            _ => 2,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            QueryError::NoProvider(_) => "no_provider",
            QueryError::NoTerms => "no_terms",
            QueryError::NoResults => "no_results",
            QueryError::EmptyTree => "empty_tree",
            QueryError::Io(_) => "io_error",
            QueryError::ContextBudgetExhausted { .. } => "context_budget_exhausted",
            QueryError::Truncated | QueryError::ContentFilter => "llm_error",
            QueryError::Llm(_) => "llm_error",
            QueryError::Parse(_) => "parse_error",
            QueryError::LowRelevance { .. } => "low_relevance",
            QueryError::InsufficientSources { .. } => "insufficient_sources",
        }
    }

    pub fn next_step(&self) -> Option<&'static str> {
        match self {
            QueryError::EmptyTree => Some("collect sources first with `bo collect <url>`"),
            QueryError::NoResults => Some(
                "collect relevant material or rephrase with terms likely to appear in the tree",
            ),
            QueryError::LowRelevance { reason, .. } => match reason {
                LowRelevanceReason::WeakMatches => Some(
                    "ask a more specific question, use more specific terms, or collect sources on this topic",
                ),
                LowRelevanceReason::GenericTerms => {
                    Some("ask with more specific terms from the topic you expect to find")
                }
            },
            QueryError::InsufficientSources { .. } => {
                Some("collect more material on this topic or rephrase your question")
            }
            _ => None,
        }
    }

    pub fn details(&self) -> serde_json::Value {
        match self {
            QueryError::LowRelevance {
                reason,
                matched_sources,
            } => json!({
                "reason": reason.as_str(),
                "matched_sources": matched_sources,
                "next_step": self.next_step().expect("low relevance has next step"),
            }),
            QueryError::InsufficientSources { leaves_consulted } => json!({
                "leaves_consulted": leaves_consulted,
                "next_step": self.next_step().expect("insufficient sources has next step"),
            }),
            QueryError::EmptyTree | QueryError::NoResults => json!({
                "next_step": self.next_step().expect("no-answer error has next step"),
            }),
            _ => json!({}),
        }
    }

    pub fn json_error(&self) -> JsonError {
        JsonError::with_details(self.code(), self.to_string(), self.details())
    }
}

// ── prepared query handoff ───────────────────────────────────────────────────

pub struct PreparedQuery {
    question: String,
    context: String,
    retrieved: Vec<RetrievedDoc>,
    leaves_consulted: usize,
    model: String,
}

// ── synthesis ────────────────────────────────────────────────────────────────

const SYNTHESIS_SYSTEM_PROMPT: &str = "\
You are a knowledge base assistant. Answer the user's question using ONLY the \
provided source material. Follow these rules strictly:

1. Cite sources using [[slug]] wikilink format inline in your prose.
2. Sources are of two kinds: leaves (raw collected documents, shown with a URL) \
   and branches (synthesized concept pages that draw connections across leaves, \
   shown as (branch)). Cite either kind by its slug.
3. If the sources don't contain enough information to answer, say so explicitly.
4. Do not invent information not present in the sources.
5. Keep your answer concise — 1 to 3 paragraphs.
6. The cited_slugs array must contain every slug you reference in your answer.";

/// Run synthesis with an injectable provider.
fn synthesize_with_provider(
    question: &str,
    context: &str,
    provider: &dyn LlmProvider,
    model: &str,
    policy: LlmCallPolicy,
) -> Result<SynthesisResponse, QueryError> {
    let user_message = format!(
        "<question>{}</question>\n\n<sources>\n{}</sources>",
        question, context
    );

    let messages = vec![
        Message::system(SYNTHESIS_SYSTEM_PROMPT),
        Message::user(user_message),
    ];

    let schema =
        serde_json::to_value(crate::engine::schema::inline_schema_for::<SynthesisResponse>())
            .unwrap();

    let response = crate::engine::llm::blocking_runtime()
        .block_on(complete_with_policy(
            provider,
            &messages,
            model,
            MAX_COMPLETION_TOKENS,
            Some(&schema),
            false,
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

// ── orchestrator ─────────────────────────────────────────────────────────────

/// Run query preflight up to, but not including, provider-backed synthesis.
/// Presentation-pure: prints nothing; the argv-facing caller emits progress.
pub fn prepare(
    tree_dir: &Path,
    question: &str,
    model: &Model,
) -> Result<PreparedQuery, QueryError> {
    let terms = extract_terms(question)?;
    let source_words = compute_context_budget(model)?;

    let retrieved = retrieve_docs(tree_dir, &terms)?;
    validate_relevance(&terms, &retrieved)?;

    let (context, consulted) = assemble_context(&retrieved, source_words);

    Ok(PreparedQuery {
        question: question.to_string(),
        context,
        retrieved,
        leaves_consulted: consulted,
        model: model.to_string(),
    })
}

/// Complete a prepared query with an injectable provider.
pub fn run_prepared_with_provider(
    prepared: PreparedQuery,
    provider: &dyn LlmProvider,
) -> Result<QueryResult, QueryError> {
    run_prepared_with_policy(prepared, provider, QUERY_LLM_POLICY)
}

pub fn run_with_provider_and_policy(
    tree_dir: &Path,
    question: &str,
    provider: &dyn LlmProvider,
    model: &Model,
    policy: LlmCallPolicy,
) -> Result<QueryResult, QueryError> {
    let prepared = prepare(tree_dir, question, model)?;
    run_prepared_with_policy(prepared, provider, policy)
}

fn run_prepared_with_policy(
    prepared: PreparedQuery,
    provider: &dyn LlmProvider,
    policy: LlmCallPolicy,
) -> Result<QueryResult, QueryError> {
    let response = synthesize_with_provider(
        &prepared.question,
        &prepared.context,
        provider,
        &prepared.model,
        policy,
    )?;

    let (answer, citations) = validate_citations(response, &prepared.retrieved);

    if citations.is_empty() {
        return Err(QueryError::InsufficientSources {
            leaves_consulted: prepared.leaves_consulted,
        });
    }

    Ok(QueryResult {
        answer,
        citations,
        model: prepared.model,
        leaves_consulted: prepared.leaves_consulted,
    })
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Render a query error to stderr in human-friendly form.
///
/// Returns the supplied `exit_code` on success, or `1` if writing failed.
pub fn render_error_human<E: std::io::Write>(
    error: &QueryError,
    stderr: &mut E,
    exit_code: i32,
) -> i32 {
    if writeln!(stderr, "error: {}", error).is_err() {
        return 1;
    }

    if let Some(next_step) = error.next_step() {
        if writeln!(stderr, "next step: {}", next_step).is_err() {
            return 1;
        }
    }

    exit_code
}

// ── journal ───────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct QueryJournalPayload<'a> {
    question: &'a str,
    answer: &'a str,
    citations: &'a [Citation],
    leaves_consulted: usize,
}

/// Record a query in the tree's journal. Best-effort: a journal failure never
/// fails the command.
pub fn journal(tree_dir: &Path, question: &str, result: &QueryResult) {
    let payload = QueryJournalPayload {
        question,
        answer: &result.answer,
        citations: &result.citations,
        leaves_consulted: result.leaves_consulted,
    };
    crate::engine::journal::append_payload(
        tree_dir,
        crate::engine::journal::Op::Query,
        Some(result.model.clone()),
        &payload,
    );
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "../tests/cli_query_tests.rs"]
mod tests;
