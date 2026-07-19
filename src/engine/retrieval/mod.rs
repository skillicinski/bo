// engine/retrieval — corpus scoring, retrieval, relevance, context assembly,
// and citation validation. Shared across search and query commands; knows
// nothing about any command's argv or output.
//
// Stage layout (this module owns shared public types + the facade re-exports):
//   terms      — term extraction, tokenization, generic-term classification.
//   scoring    — document loading, IDF scoring, leaf/branch ranking.
//   relevance  — diagnostics computation + low-relevance gating.
//   context    — model budget + source-context assembly.
//   citations  — synthesis response + wikilink/citation validation.
//
// Dependency direction: scoring -> relevance; stages depend only on `terms`
// and on shared types in this module. Re-exports keep caller addresses stable.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod citations;
mod context;
mod relevance;
mod scoring;
mod terms;

// ── shared public types ──────────────────────────────────────────────────────

/// Whether a scored document is a raw leaf or a synthesized branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Leaf,
    Branch,
}

/// A document (leaf or branch) scored against a set of terms.
pub(crate) struct ScoredDoc {
    pub(crate) kind: DocKind,
    pub(crate) slug: String,
    pub(crate) file: String,
    pub(crate) title: String,
    pub(crate) url: String,
    pub(crate) summary: String,
    pub(crate) body: String,
    pub(crate) score: f64,
}

/// Why a retrieval result was judged too weak to synthesize against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LowRelevanceReason {
    WeakMatches,
    GenericTerms,
}

impl LowRelevanceReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            LowRelevanceReason::WeakMatches => "weak_matches",
            LowRelevanceReason::GenericTerms => "generic_query",
        }
    }
}

/// Capability-layer retrieval failures. Carries semantic state only; the CLI
/// renders human messages and exit codes from these.
#[derive(Debug)]
pub enum RetrievalError {
    /// Could not extract meaningful terms from the question
    NoTerms,
    /// No relevant sources found in the tree
    NoResults,
    /// Tree has no leaves
    EmptyTree,
    /// State read or file I/O error
    Io(String),
    /// Known model has too little context after reserved prompt/completion budget
    ContextBudgetExhausted {
        model: String,
        context_tokens: usize,
        reserved_tokens: usize,
    },
    /// Retrieved matches are too weak or generic to support synthesis
    LowRelevance {
        reason: LowRelevanceReason,
        matched_sources: usize,
    },
}

/// A validated citation to a retrieved source.
#[derive(Debug, Clone, Serialize)]
pub struct Citation {
    pub slug: String,
    pub title: String,
    pub file: String,
}

/// A document retrieved for synthesis, with per-field relevance diagnostics.
#[derive(Debug, Clone)]
pub struct RetrievedDoc {
    pub kind: DocKind,
    pub slug: String,
    pub title: String,
    pub url: String,
    pub file: String,
    pub summary: String,
    pub body: String,
    pub score: f64,
    pub diagnostics: RetrievalDiagnostics,
}

#[derive(Debug, Clone, Default)]
pub struct RetrievalDiagnostics {
    pub matched_terms: usize,
    pub matched_non_generic_terms: usize,
    pub total_hits: usize,
    pub title_hits: usize,
    pub summary_hits: usize,
    pub body_hits: usize,
    pub title_summary_non_generic_hits: usize,
    pub token_count: usize,
}

/// The structured synthesis response deserialized from the model output.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SynthesisResponse {
    pub answer: String,
    pub cited_slugs: Vec<String>,
}

/// Max completion tokens for a synthesis call. Shared across the capability
/// boundary: the CLI's synthesis step and the context-budget computation both
/// reserve this much output capacity.
pub const MAX_COMPLETION_TOKENS: u32 = 2048;

// ── public facade re-exports ────────────────────────────────────────────────

pub use citations::validate_citations;
pub use context::{assemble_context, compute_context_budget};
pub use relevance::validate_relevance;
pub use scoring::retrieve_docs;
pub use terms::extract_terms;
// crate-visible: cli::synthesize::cluster::discovery tokenizes search text here.
pub(crate) use terms::tokenize;
