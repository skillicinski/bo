// engine/retrieval — shared corpus scoring for search and query commands.

use crate::domain::frontmatter;
use crate::domain::manifest::Manifest;
use crate::engine::summary;
use std::fs;
use std::path::Path;

// ── public types ─────────────────────────────────────────────────────────────

/// How to score term matches against leaves.
pub enum ScoringPolicy {
    /// AND semantics: leaf must contain ALL terms. Score = sum of occurrences * 1000 / word_count.
    AllTermsRequired,
    /// OR semantics: any term hit counts. Score = sum of occurrences * 1000 / word_count.
    AnyTermCounts,
}

/// A leaf scored against a set of query terms.
pub struct ScoredLeaf {
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
/// Returns only leaves with score > 0, unsorted (caller sorts).
pub fn score_corpus(
    tree_dir: &Path,
    manifest: &Manifest,
    terms: &[String],
    policy: ScoringPolicy,
) -> Vec<ScoredLeaf> {
    let mut results = Vec::new();

    for (index_position, leaf) in manifest.leaves.iter().enumerate() {
        let path = tree_dir.join(&leaf.file);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let body = match frontmatter::parse(&content) {
            Ok((_, body)) => body,
            Err(_) => match policy {
                // search: fall back to full content for malformed files
                ScoringPolicy::AllTermsRequired => content.clone(),
                // query: skip malformed leaves entirely
                ScoringPolicy::AnyTermCounts => continue,
            },
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

        let dominated = match policy {
            ScoringPolicy::AllTermsRequired => {
                // All terms must appear in the searchable text
                !terms.iter().all(|term| searchable.contains(term.as_str()))
            }
            ScoringPolicy::AnyTermCounts => total_hits == 0,
        };

        if dominated {
            continue;
        }

        let score = (total_hits as f64 * 1000.0) / word_count as f64;

        let collected_at = {
            let s = leaf.collected_at.to_rfc3339_millis();
            if s.trim().is_empty() {
                None
            } else {
                Some(s)
            }
        };

        results.push(ScoredLeaf {
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

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "../tests/engine_retrieval_tests.rs"]
mod tests;
