// engine/retrieval — corpus scoring, retrieval, relevance, context assembly,
// and citation validation. Shared across search and query commands; knows
// nothing about any command's argv or output.

use crate::domain::frontmatter;
use crate::domain::manifest::Manifest;
use crate::domain::tree::TreeRuntimeState;
use crate::engine::llm::Model;
use crate::engine::summary;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

// ── constants ────────────────────────────────────────────────────────────────

const RETRIEVAL_TOP_K: usize = 10;
const DEPTH_TOP_K: usize = 5;
/// Max completion tokens for a synthesis call. Shared across the capability
/// boundary: the CLI's synthesis step and the context-budget computation both
/// reserve this much output capacity.
pub const MAX_COMPLETION_TOKENS: u32 = 2048;
const PROMPT_OVERHEAD_TOKENS: usize = 4096;
const MIN_SOURCE_WORDS: usize = 1000;
const TOKENS_TO_WORDS_NUMERATOR: usize = 3;
const TOKENS_TO_WORDS_DENOMINATOR: usize = 4;
const MIN_SINGLE_TERM_DENSITY: f64 = 20.0;
const MIN_MULTI_TERM_DENSITY: f64 = 8.0;
const MOSTLY_GENERIC_RATIO_NUMERATOR: usize = 2;
const MOSTLY_GENERIC_RATIO_DENOMINATOR: usize = 3;

const STOP_WORDS: &[&str] = &[
    "what", "which", "who", "whom", "where", "when", "why", "how", "is", "are", "was", "were",
    "am", "do", "does", "did", "has", "have", "had", "can", "could", "would", "should", "will",
    "shall", "the", "a", "an", "of", "in", "on", "at", "to", "for", "with", "by", "from", "about",
    "between", "and", "or", "but", "not", "no", "if", "then", "than", "that", "this", "these",
    "those", "it", "its", "be", "been", "being", "my", "your", "our", "their", "me", "you", "us",
    "them", "he", "she", "we", "they", "his", "her",
];

// ── public types ─────────────────────────────────────────────────────────────

/// Whether a scored document is a raw leaf or a synthesized branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Leaf,
    Branch,
}

/// A document (leaf or branch) scored against a set of terms.
pub struct ScoredDoc {
    pub kind: DocKind,
    pub slug: String,
    pub file: String,
    pub title: String,
    pub url: String,
    pub summary: String,
    pub body: String,
    pub score: f64,
    pub collected_at: Option<String>,
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
    /// Index read or file I/O error
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

// ── shared scoring ───────────────────────────────────────────────────────────

/// A document reduced to the fields the scorer needs. Leaf and branch records
/// map into this; the scoring math is then identical for both.
struct Scorable {
    kind: DocKind,
    slug: String,
    file: String,
    title: String,
    url: String,
    summary: String,
    body: String,
    collected_at: Option<String>,
}

/// Score a stream of documents against `terms` using token-level matching
/// with IDF weighting, normalized by document length. Matches at the token
/// level (not substring), weights each term by smoothed IDF: `1 + log(N/df)`,
/// then divides by token count. OR semantics.
/// Returns only docs with score > 0, unsorted (caller sorts).
fn score_candidates(
    candidates: impl Iterator<Item = Scorable>,
    terms: &[String],
) -> Vec<ScoredDoc> {
    // Tokenize each doc once
    let docs: Vec<(Scorable, Vec<String>)> = candidates
        .map(|c| {
            let searchable = format!("{} {} {}", c.title, c.summary, c.body).to_lowercase();
            let tokens = tokenize(&searchable);
            (c, tokens)
        })
        .collect();

    let n = docs.len();
    if n == 0 || terms.is_empty() {
        return Vec::new();
    }

    // Document frequency from pre-tokenized docs
    let df: Vec<usize> = terms
        .iter()
        .map(|term| {
            docs.iter()
                .filter(|(_, tokens)| count_term_hits_in_tokens(tokens, term) > 0)
                .count()
        })
        .collect();

    docs.into_iter()
        .filter_map(|(c, tokens)| {
            let token_count = tokens.len();
            if token_count == 0 {
                return None;
            }

            let raw_score: f64 = terms
                .iter()
                .zip(df.iter())
                .map(|(term, &df)| {
                    if df == 0 {
                        return 0.0;
                    }
                    let hits = count_term_hits_in_tokens(&tokens, term);
                    if hits == 0 {
                        return 0.0;
                    }
                    let idf = 1.0 + (n as f64 / df as f64).ln();
                    (hits as f64) * idf
                })
                .sum();

            if raw_score == 0.0 {
                return None;
            }

            let score = raw_score / token_count as f64;

            Some(ScoredDoc {
                kind: c.kind,
                slug: c.slug,
                file: c.file,
                title: c.title,
                url: c.url,
                summary: c.summary,
                body: c.body,
                score,
                collected_at: c.collected_at,
            })
        })
        .collect()
}

// ── public API: corpus scoring ───────────────────────────────────────────────

fn iter_leaves<'a>(
    tree_dir: &'a Path,
    manifest: &'a Manifest,
) -> impl Iterator<Item = Scorable> + 'a {
    manifest.leaves.iter().filter_map(move |leaf| {
        let body = read_body(tree_dir, &leaf.file)?;
        let title = leaf
            .title
            .as_ref()
            .map(|t| t.as_str().to_string())
            .unwrap_or_else(|| leaf.file.clone());
        let summary = leaf
            .summary
            .clone()
            .unwrap_or_else(|| summary::generate_fallback(&body));
        Some(Scorable {
            kind: DocKind::Leaf,
            slug: leaf.slug.as_str().to_string(),
            file: leaf.file.clone(),
            title,
            url: leaf.url.as_str().to_string(),
            summary,
            body,
            collected_at: Some(leaf.collected_at.to_rfc3339_millis()),
        })
    })
}

fn iter_branches<'a>(
    tree_dir: &'a Path,
    manifest: &'a Manifest,
) -> impl Iterator<Item = Scorable> + 'a {
    manifest.branches.iter().filter_map(move |branch| {
        let body = read_body(tree_dir, &branch.file)?;
        let title = branch.title.as_str().to_string();
        let summary = summary::generate_fallback(&body);
        Some(Scorable {
            kind: DocKind::Branch,
            slug: branch.slug.as_str().to_string(),
            file: branch.file.clone(),
            title,
            url: String::new(),
            summary,
            body,
            collected_at: None,
        })
    })
}

/// Score all leaves in a manifest against the given terms.
///
/// Reads leaf files from `tree_dir`. Skips missing/unreadable/malformed files.
pub fn score_corpus(tree_dir: &Path, manifest: &Manifest, terms: &[String]) -> Vec<ScoredDoc> {
    score_candidates(iter_leaves(tree_dir, manifest), terms)
}

/// Score all branches in a manifest against the given terms.
///
/// A branch is a synthesized concept page; its body is the cross-source
/// synthesis, so it is scored as a single searchable document. Branches have
/// no URL (`url` empty, `collected_at` None). This makes compile's synthesized
/// output reachable at retrieval time — without it, only raw leaves are visible
/// and the compiled branches are invisible.
pub fn score_branches(tree_dir: &Path, manifest: &Manifest, terms: &[String]) -> Vec<ScoredDoc> {
    score_candidates(iter_branches(tree_dir, manifest), terms)
}

/// Read a file under `tree_dir` and return its post-frontmatter body, or None
/// if missing/unreadable/malformed.
fn read_body(tree_dir: &Path, file: &str) -> Option<String> {
    let content = fs::read_to_string(tree_dir.join(file)).ok()?;
    frontmatter::parse(&content).ok().map(|(_, body)| body)
}

// ── term extraction ──────────────────────────────────────────────────────────

/// Extract meaningful search terms from a natural-language question.
/// Strips stop words, possessives, boundary punctuation, and terms < 2 chars.
pub fn extract_terms(question: &str) -> Result<Vec<String>, RetrievalError> {
    let terms: Vec<String> = question
        .split_whitespace()
        .map(strip_punctuation)
        .map(|w| strip_possessive(&w))
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 2)
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .collect();

    if terms.is_empty() {
        return Err(RetrievalError::NoTerms);
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

fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in input.chars() {
        for lower in ch.to_lowercase() {
            if lower.is_alphanumeric() {
                current.push(lower);
            } else if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn unique_terms(terms: &[String]) -> Vec<&str> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();

    for term in terms {
        if seen.insert(term.as_str()) {
            unique.push(term.as_str());
        }
    }

    unique
}

fn count_term_hits_in_tokens(tokens: &[String], term: &str) -> usize {
    let term_tokens = tokenize(term);
    match term_tokens.len() {
        0 => 0,
        1 => tokens
            .iter()
            .filter(|token| token.as_str() == term_tokens[0].as_str())
            .count(),
        n if n <= tokens.len() => tokens
            .windows(n)
            .filter(|window| {
                window
                    .iter()
                    .map(String::as_str)
                    .eq(term_tokens.iter().map(String::as_str))
            })
            .count(),
        _ => 0,
    }
}

fn is_generic_term(term: &str) -> bool {
    matches!(
        term,
        "important"
            | "system"
            | "systems"
            | "pattern"
            | "patterns"
            | "concept"
            | "concepts"
            | "model"
            | "models"
            | "approach"
            | "approaches"
            | "method"
            | "methods"
            | "topic"
            | "topics"
            | "source"
            | "sources"
            | "information"
            | "details"
            | "example"
            | "examples"
            | "data"
            | "content"
            | "use"
            | "uses"
            | "using"
            | "used"
            | "work"
            | "works"
            | "benefit"
            | "benefits"
            | "tradeoff"
            | "tradeoffs"
            | "good"
            | "bad"
            | "best"
            | "common"
            | "general"
            | "overview"
            | "summary"
            | "guide"
    )
}

fn compute_retrieval_diagnostics(
    title: &str,
    summary: &str,
    body: &str,
    terms: &[String],
) -> RetrievalDiagnostics {
    let title_tokens = tokenize(title);
    let summary_tokens = tokenize(summary);
    let body_tokens = tokenize(body);
    let unique_terms = unique_terms(terms);

    let mut diagnostics = RetrievalDiagnostics {
        token_count: title_tokens.len() + summary_tokens.len() + body_tokens.len(),
        ..RetrievalDiagnostics::default()
    };

    for term in unique_terms {
        let title_hits = count_term_hits_in_tokens(&title_tokens, term);
        let summary_hits = count_term_hits_in_tokens(&summary_tokens, term);
        let body_hits = count_term_hits_in_tokens(&body_tokens, term);
        let term_hits = title_hits + summary_hits + body_hits;

        if term_hits > 0 {
            diagnostics.matched_terms += 1;
            if !is_generic_term(term) {
                diagnostics.matched_non_generic_terms += 1;
            }
        }

        if !is_generic_term(term) {
            diagnostics.title_summary_non_generic_hits += title_hits + summary_hits;
        }

        diagnostics.title_hits += title_hits;
        diagnostics.summary_hits += summary_hits;
        diagnostics.body_hits += body_hits;
        diagnostics.total_hits += term_hits;
    }

    diagnostics
}

// ── retrieval ────────────────────────────────────────────────────────────────

/// Retrieve top-k documents (leaves and branches) scored by term density
/// (OR semantics). Branches are synthesized concept pages from `bo compile`;
/// including them makes compile's output reachable at retrieval time.
pub fn retrieve_docs(
    tree_dir: &Path,
    terms: &[String],
) -> Result<Vec<RetrievedDoc>, RetrievalError> {
    let manifest = match crate::engine::manifest::runtime_state(tree_dir) {
        Ok(TreeRuntimeState::Initialized(manifest)) => manifest,
        Ok(TreeRuntimeState::FreshSeeded) => return Err(RetrievalError::EmptyTree),
        Ok(TreeRuntimeState::MissingManifest) => {
            return Err(RetrievalError::Io(format!(
                "manifest: {}",
                crate::domain::manifest::ManifestError::TreeNotInitialized
            )));
        }
        Err(e) => return Err(RetrievalError::Io(format!("manifest: {}", e))),
    };

    if manifest.leaves.is_empty() {
        return Err(RetrievalError::EmptyTree);
    }

    let scored = score_candidates(
        iter_leaves(tree_dir, &manifest).chain(iter_branches(tree_dir, &manifest)),
        terms,
    );

    let mut scored: Vec<RetrievedDoc> = scored
        .into_iter()
        .map(|s| {
            let diagnostics = compute_retrieval_diagnostics(&s.title, &s.summary, &s.body, terms);
            // ponytail: field-by-field copy from ScoredDoc rather than embedding it —
            // consumers (assemble_context, validate_citations) read flat fields, and
            // embedding would push a `.scored.` prefix onto every read site.
            RetrievedDoc {
                kind: s.kind,
                slug: s.slug,
                title: s.title,
                url: s.url,
                file: s.file,
                summary: s.summary,
                body: s.body,
                score: s.score,
                diagnostics,
            }
        })
        .collect();

    if scored.is_empty() {
        return Err(RetrievalError::NoResults);
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

// ── relevance validation ─────────────────────────────────────────────────────

pub fn validate_relevance(terms: &[String], docs: &[RetrievedDoc]) -> Result<(), RetrievalError> {
    if docs.is_empty() {
        return Err(RetrievalError::NoResults);
    }

    let matched_sources = docs.len();

    if is_mostly_generic(terms) && !docs.iter().any(|doc| is_focused_generic_match(doc, terms)) {
        return Err(RetrievalError::LowRelevance {
            reason: LowRelevanceReason::GenericTerms,
            matched_sources,
        });
    }

    if !docs.iter().any(|doc| is_strong_relevance_match(doc, terms)) {
        return Err(RetrievalError::LowRelevance {
            reason: LowRelevanceReason::WeakMatches,
            matched_sources,
        });
    }

    Ok(())
}

fn is_mostly_generic(terms: &[String]) -> bool {
    let unique_terms = unique_terms(terms);
    if unique_terms.is_empty() {
        return false;
    }

    let generic_terms = unique_terms
        .iter()
        .filter(|term| is_generic_term(term))
        .count();

    generic_terms * MOSTLY_GENERIC_RATIO_DENOMINATOR
        >= unique_terms.len() * MOSTLY_GENERIC_RATIO_NUMERATOR
}

fn is_focused_generic_match(doc: &RetrievedDoc, terms: &[String]) -> bool {
    let unique_term_count = unique_terms(terms).len();
    if unique_term_count == 0 {
        return false;
    }

    let required_terms = unique_term_count.min(2);
    let title_summary_hits = doc.diagnostics.title_hits + doc.diagnostics.summary_hits;

    doc.diagnostics.matched_terms >= required_terms && title_summary_hits >= required_terms
}

fn is_strong_relevance_match(doc: &RetrievedDoc, terms: &[String]) -> bool {
    let diagnostics = &doc.diagnostics;
    if diagnostics.matched_terms == 0 || diagnostics.total_hits == 0 {
        return false;
    }

    let unique_terms = unique_terms(terms);
    let unique_term_count = unique_terms.len();
    let non_generic_term_count = unique_terms
        .iter()
        .filter(|term| !is_generic_term(term))
        .count();
    let title_summary_hits = diagnostics.title_hits + diagnostics.summary_hits;
    let density = if diagnostics.token_count == 0 {
        0.0
    } else {
        (diagnostics.total_hits as f64 * 1000.0) / diagnostics.token_count as f64
    };

    if unique_term_count == 1 {
        let term = unique_terms[0];
        return !is_generic_term(term)
            && (title_summary_hits > 0
                || diagnostics.total_hits >= 2
                || density >= MIN_SINGLE_TERM_DENSITY);
    }

    if non_generic_term_count == 1
        && diagnostics.matched_non_generic_terms == 1
        && diagnostics.title_summary_non_generic_hits > 0
    {
        return true;
    }

    if non_generic_term_count > 1
        && diagnostics.matched_non_generic_terms >= non_generic_term_count.min(2)
        && (diagnostics.title_summary_non_generic_hits > 0 || density >= MIN_MULTI_TERM_DENSITY)
    {
        return true;
    }

    diagnostics.matched_terms >= unique_term_count.min(2)
        && (title_summary_hits > 0 || density >= MIN_MULTI_TERM_DENSITY)
}

// ── context assembly ─────────────────────────────────────────────────────────

pub fn compute_context_budget(model: &Model) -> Result<usize, RetrievalError> {
    compute_context_budget_from_tokens(model.as_str(), model.context_tokens())
}

pub fn compute_context_budget_from_tokens(
    model: &str,
    context_tokens: usize,
) -> Result<usize, RetrievalError> {
    let reserved_tokens = PROMPT_OVERHEAD_TOKENS + MAX_COMPLETION_TOKENS as usize;
    if context_tokens <= reserved_tokens {
        return Err(RetrievalError::ContextBudgetExhausted {
            model: model.to_string(),
            context_tokens,
            reserved_tokens,
        });
    }

    let source_words = ((context_tokens - reserved_tokens) * TOKENS_TO_WORDS_NUMERATOR)
        / TOKENS_TO_WORDS_DENOMINATOR;

    if source_words < MIN_SOURCE_WORDS {
        return Err(RetrievalError::ContextBudgetExhausted {
            model: model.to_string(),
            context_tokens,
            reserved_tokens,
        });
    }

    Ok(source_words)
}

/// Assemble LLM context from retrieved documents (leaves and branches).
/// Returns (context_string, consulted_count).
pub fn assemble_context(docs: &[RetrievedDoc], source_word_budget: usize) -> (String, usize) {
    let mut context = String::new();
    let mut word_budget = source_word_budget;
    let mut consulted = 0;

    // Breadth tier: all retrieved docs get summary context
    context.push_str("## Available sources\n\n");
    for doc in docs {
        // Branches are synthesized concept pages with no URL; leaves are raw
        // sources collected from a URL. The label tells the LLM which is which.
        let origin = match doc.kind {
            DocKind::Leaf => format!("({})", doc.url),
            DocKind::Branch => "(branch)".to_string(),
        };
        let entry = format!(
            "- [[{}]] — {} {}
  Summary: {}\n\n",
            doc.slug, doc.title, origin, doc.summary
        );
        let words = entry.split_whitespace().count();
        if words > word_budget {
            break;
        }
        context.push_str(&entry);
        word_budget = word_budget.saturating_sub(words);
    }

    // Depth tier: top-k get full body
    let depth_count = docs.len().min(DEPTH_TOP_K);
    if depth_count > 0 {
        context.push_str("## Full source content\n\n");
    }
    for doc in docs.iter().take(depth_count) {
        let body_words: Vec<&str> = doc.body.split_whitespace().collect();
        let usable_words = body_words.len().min(word_budget);
        if usable_words == 0 {
            break;
        }
        let truncated_body: String = body_words[..usable_words].join(" ");

        let entry = format!(
            "### [[{}]] — {}\n\n{}\n\n",
            doc.slug, doc.title, truncated_body
        );
        let entry_words = entry.split_whitespace().count();
        context.push_str(&entry);
        word_budget = word_budget.saturating_sub(entry_words);
        consulted += 1;
    }

    (context, consulted)
}

// ── citation validation ──────────────────────────────────────────────────────

/// Validate citations against the retrieval set.
/// Strips invalid slugs from cited_slugs and removes invalid [[slug]] from prose.
pub fn validate_citations(
    response: SynthesisResponse,
    retrieved: &[RetrievedDoc],
) -> (String, Vec<Citation>) {
    let valid_slugs: HashSet<&str> = retrieved.iter().map(|l| l.slug.as_str()).collect();

    let (answer, prose_slugs) =
        sanitize_wikilinks_and_collect_valid(&response.answer, &valid_slugs);

    let mut ordered_slugs: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for slug in prose_slugs.into_iter().chain(response.cited_slugs) {
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

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "../tests/engine_retrieval_tests.rs"]
mod tests;
