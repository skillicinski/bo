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
    pub index_position: usize,
    pub collected_at: Option<String>,
}

// ── public API ───────────────────────────────────────────────────────────────

/// Score all leaves in a manifest against the given terms.
///
/// Reads leaf files from `tree_dir`. Skips missing/unreadable/malformed files.
/// Uses OR semantics: any term hit counts. Score = sum of occurrences * 1000 / word_count.
/// Returns only leaves with score > 0, unsorted (caller sorts).
pub fn score_corpus(tree_dir: &Path, manifest: &Manifest, terms: &[String]) -> Vec<ScoredDoc> {
    let mut results = Vec::new();

    for (index_position, leaf) in manifest.leaves.iter().enumerate() {
        let path = tree_dir.join(&leaf.file);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let body = match frontmatter::parse(&content) {
            Ok((_, body)) => body,
            Err(_) => continue,
        };

        let title = if leaf.title.as_str().trim().is_empty() {
            leaf.file.clone()
        } else {
            leaf.title.as_str().to_string()
        };
        let url = leaf.url.as_str().to_string();
        let summary = leaf
            .summary
            .clone()
            .unwrap_or_else(|| summary::generate_fallback(&body));

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

        let collected_at = Some(leaf.collected_at.to_rfc3339_millis());

        results.push(ScoredDoc {
            kind: DocKind::Leaf,
            slug: leaf.slug.as_str().to_string(),
            file: leaf.file.clone(),
            title,
            url,
            summary,
            body,
            score,
            index_position,
            collected_at,
        });
    }

    results
}

/// Score all branches in a manifest against the given terms.
///
/// A branch is a synthesized concept page; its body is the cross-source
/// synthesis, so it is scored as a single searchable document. Branches have
/// no URL; `url` is empty and `collected_at` is `None`. This makes compile's
/// synthesized output reachable at query time — without it, `bo query` sees
/// only raw leaves and the compiled branches are invisible.
pub fn score_branches(tree_dir: &Path, manifest: &Manifest, terms: &[String]) -> Vec<ScoredDoc> {
    let mut results = Vec::new();

    for (index_position, branch) in manifest.branches.iter().enumerate() {
        let path = tree_dir.join(&branch.file);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let body = match frontmatter::parse(&content) {
            Ok((_, body)) => body,
            Err(_) => continue,
        };

        let title = branch.title.as_str().to_string();
        let summary = summary::generate_fallback(&body);

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

        results.push(ScoredDoc {
            kind: DocKind::Branch,
            slug: branch.slug.as_str().to_string(),
            file: branch.file.clone(),
            title,
            url: String::new(),
            summary,
            body,
            score,
            index_position,
            collected_at: None,
        });
    }

    results
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "../tests/engine_retrieval_tests.rs"]
mod tests;
