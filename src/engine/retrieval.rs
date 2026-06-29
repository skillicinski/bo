// engine/retrieval — shared corpus scoring for search and query commands.

use crate::domain::frontmatter;
use crate::domain::manifest::Manifest;
use crate::engine::summary;
use std::fs;
use std::path::Path;

// ── public types ─────────────────────────────────────────────────────────────

/// Whether a scored document is a raw leaf or a synthesized branch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocKind {
    Leaf,
    Branch,
}

/// A document (leaf or branch) scored against a set of query terms.
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

/// Score a stream of documents against `terms`.
///
/// // ponytail: substring density, not BM25/TF-IDF. `str::matches` counts
/// // overlapping substring occurrences in lowercased `title summary body`,
/// // so "rust" hits inside "trust". Good enough at bo's scale (<~1k leaves);
/// // if relevance degrades, move to token-aware scoring (the diagnostics path
/// // in query.rs already tokenizes — reuse `count_term_hits_in_tokens` there).
/// // OR semantics: any term hit counts. Score = total_hits * 1000 / word_count.
/// // Returns only docs with score > 0, unsorted (caller sorts).
fn score_candidates(
    candidates: impl Iterator<Item = Scorable>,
    terms: &[String],
) -> Vec<ScoredDoc> {
    candidates
        .filter_map(|c| {
            let searchable = format!("{} {} {}", c.title, c.summary, c.body).to_lowercase();
            let word_count = searchable.split_whitespace().count();
            if word_count == 0 {
                return None;
            }
            let total_hits: usize = terms
                .iter()
                .map(|term| searchable.matches(term.as_str()).count())
                .sum();
            if total_hits == 0 {
                return None;
            }
            let score = (total_hits as f64 * 1000.0) / word_count as f64;
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

// ── public API ───────────────────────────────────────────────────────────────

/// Score all leaves in a manifest against the given terms.
///
/// Reads leaf files from `tree_dir`. Skips missing/unreadable/malformed files.
pub fn score_corpus(tree_dir: &Path, manifest: &Manifest, terms: &[String]) -> Vec<ScoredDoc> {
    let candidates = manifest.leaves.iter().filter_map(|leaf| {
        let body = read_body(tree_dir, &leaf.file)?;
        let title = if leaf.title.as_str().trim().is_empty() {
            leaf.file.clone()
        } else {
            leaf.title.as_str().to_string()
        };
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
    });
    score_candidates(candidates, terms)
}

/// Score all branches in a manifest against the given terms.
///
/// A branch is a synthesized concept page; its body is the cross-source
/// synthesis, so it is scored as a single searchable document. Branches have
/// no URL (`url` empty, `collected_at` None). This makes compile's synthesized
/// output reachable at query time — without it, `bo query` sees only raw
/// leaves and the compiled branches are invisible.
pub fn score_branches(tree_dir: &Path, manifest: &Manifest, terms: &[String]) -> Vec<ScoredDoc> {
    let candidates = manifest.branches.iter().filter_map(|branch| {
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
    });
    score_candidates(candidates, terms)
}

/// Read a file under `tree_dir` and return its post-frontmatter body, or None
/// if missing/unreadable/malformed.
fn read_body(tree_dir: &Path, file: &str) -> Option<String> {
    let content = fs::read_to_string(tree_dir.join(file)).ok()?;
    frontmatter::parse(&content).ok().map(|(_, body)| body)
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "../tests/engine_retrieval_tests.rs"]
mod tests;
