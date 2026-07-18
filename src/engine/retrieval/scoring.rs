// retrieval/scoring — document loading, IDF scoring, leaf/branch ranking.

use super::relevance::compute_retrieval_diagnostics;
use super::terms::{count_term_hits_in_tokens, tokenize};
use super::{DocKind, RetrievalError, RetrievedDoc, ScoredDoc};
use crate::domain::frontmatter;
use crate::domain::manifest::Manifest;
use crate::domain::tree::TreeRuntimeState;
use crate::engine::summary;
use std::fs;
use std::path::Path;

const RETRIEVAL_TOP_K: usize = 10;

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
///
/// Uses per-call IDF — scores are corpus-relative; use `retrieve_docs` for
/// combined leaf+branch scoring.
pub fn score_corpus(tree_dir: &Path, manifest: &Manifest, terms: &[String]) -> Vec<ScoredDoc> {
    score_candidates(iter_leaves(tree_dir, manifest), terms)
}

/// Read a file under `tree_dir` and return its post-frontmatter body, or None
/// if missing/unreadable/malformed.
fn read_body(tree_dir: &Path, file: &str) -> Option<String> {
    let content = fs::read_to_string(tree_dir.join(file)).ok()?;
    frontmatter::parse(&content).ok().map(|(_, body)| body)
}

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

#[cfg(test)]
#[path = "../../tests/engine_retrieval/scoring_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../tests/engine_retrieval/retrieve_tests.rs"]
mod retrieve_tests;
