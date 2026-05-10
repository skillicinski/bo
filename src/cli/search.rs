// bo search — deterministic lexical search over collected leaves.

use crate::domain::{frontmatter, index};
use chrono::{DateTime, FixedOffset};
use serde::Serialize;
use std::cmp::Ordering;
use std::fmt;
use std::fs;
use std::path::Path;

// ── constants ────────────────────────────────────────────────────────────────

const PAGE_SIZE: usize = 5;
const SNIPPET_RADIUS: usize = 80;
const FALLBACK_SNIPPET_LEN: usize = 160;

// ── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Each entry is a term or phrase (quoted args become single entries).
    /// All lowercased at parse time.
    pub terms: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub page: usize,
    pub recent: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub total_results: usize,
    pub page: usize,
    pub total_pages: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub file: String,
    pub title: String,
    pub snippet: String,
    #[serde(skip)]
    pub score: usize,
    pub collected_at: Option<String>,
}

#[derive(Debug)]
pub enum SearchError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SearchError::Io(e) => write!(f, "I/O error: {}", e),
            SearchError::Json(e) => write!(f, "JSON error: {}", e),
        }
    }
}

impl From<std::io::Error> for SearchError {
    fn from(e: std::io::Error) -> Self {
        SearchError::Io(e)
    }
}

impl From<serde_json::Error> for SearchError {
    fn from(e: serde_json::Error) -> Self {
        SearchError::Json(e)
    }
}

// ── internal types ───────────────────────────────────────────────────────────

struct ScoredLeaf {
    file: String,
    title: String,
    body: String,
    score: usize,
    collected_at: Option<String>,
    index_position: usize,
}

// ── core matching ────────────────────────────────────────────────────────────

/// Returns true if every term appears as a substring of content.
/// Both content and terms must be pre-lowercased.
fn matches_all_terms(content_lower: &str, terms_lower: &[String]) -> bool {
    terms_lower
        .iter()
        .all(|term| content_lower.contains(term.as_str()))
}

/// Per-mille density normalized by word count: (sum of occurrences * 1000) / word_count.
/// Both content and terms must be pre-lowercased.
/// Returns 0 if content has no words.
fn score_relevance(content_lower: &str, terms_lower: &[String]) -> usize {
    let word_count = content_lower.split_whitespace().count();
    if word_count == 0 {
        return 0;
    }
    let total: usize = terms_lower
        .iter()
        .map(|term| content_lower.matches(term.as_str()).count())
        .sum();
    (total * 1000) / word_count
}

// ── snippet extraction ───────────────────────────────────────────────────────

/// Extract a KWIC snippet from body around the first occurrence of any term.
/// Returns ±radius chars around the match, with `…` prepended/appended if truncated.
/// Newlines are collapsed to single space.
/// If no term is found in body, returns the first FALLBACK_SNIPPET_LEN chars.
fn extract_snippet(body: &str, terms_lower: &[String], radius: usize) -> String {
    let body_lower = body.to_lowercase();

    // Find the earliest occurrence of any term in the body
    let earliest = terms_lower
        .iter()
        .filter_map(|term| body_lower.find(term.as_str()).map(|pos| (pos, term.len())))
        .min_by_key(|(pos, _)| *pos);

    let raw = match earliest {
        Some((byte_pos, term_byte_len)) => extract_window(body, byte_pos, term_byte_len, radius),
        None => {
            // Fallback: first FALLBACK_SNIPPET_LEN chars of body
            extract_prefix(body, FALLBACK_SNIPPET_LEN)
        }
    };

    collapse_newlines(&raw)
}

/// Extract a character window around a byte position in the original string.
fn extract_window(text: &str, byte_pos: usize, term_byte_len: usize, radius: usize) -> String {
    let char_indices: Vec<(usize, char)> = text.char_indices().collect();

    // Find the char index corresponding to byte_pos
    let match_char_idx = char_indices
        .iter()
        .position(|(bi, _)| *bi >= byte_pos)
        .unwrap_or(char_indices.len());

    // Find the char index for end of match
    let match_end_byte = byte_pos + term_byte_len;
    let match_end_char_idx = char_indices
        .iter()
        .position(|(bi, _)| *bi >= match_end_byte)
        .unwrap_or(char_indices.len());

    let start_char = match_char_idx.saturating_sub(radius);
    let end_char = (match_end_char_idx + radius).min(char_indices.len());

    let start_byte = char_indices[start_char].0;
    let end_byte = if end_char >= char_indices.len() {
        text.len()
    } else {
        char_indices[end_char].0
    };

    let slice = &text[start_byte..end_byte];

    let mut result = String::new();
    if start_char > 0 {
        result.push('…');
    }
    result.push_str(slice);
    if end_byte < text.len() {
        result.push('…');
    }
    result
}

/// Extract the first `max_chars` characters from text, appending `…` if truncated.
fn extract_prefix(text: &str, max_chars: usize) -> String {
    let char_indices: Vec<(usize, char)> = text.char_indices().collect();
    if char_indices.len() <= max_chars {
        return text.to_string();
    }
    let end_byte = char_indices[max_chars].0;
    let mut result = text[..end_byte].to_string();
    result.push('…');
    result
}

/// Collapse runs of whitespace (including newlines) into single spaces.
fn collapse_newlines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_ascii_whitespace() {
            if !prev_ws {
                result.push(' ');
            }
            prev_ws = true;
        } else {
            result.push(ch);
            prev_ws = false;
        }
    }
    result
}

// ── orchestration ────────────────────────────────────────────────────────────

/// Search all leaves in the tree for matching terms.
pub fn search_leaves(
    tree_dir: &Path,
    query: &SearchQuery,
    options: &SearchOptions,
) -> Result<SearchResult, SearchError> {
    let index_path = tree_dir.join("index.jsonl");
    let entries = index::read_index(&index_path)?;

    let mut scored: Vec<ScoredLeaf> = Vec::new();

    for (index_position, entry) in entries.iter().enumerate() {
        let path = tree_dir.join(&entry.file);
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue, // skip missing/unreadable files
        };

        let content_lower = content.to_lowercase();
        if !matches_all_terms(&content_lower, &query.terms) {
            continue;
        }

        let score = score_relevance(&content_lower, &query.terms);

        // Extract title and body from frontmatter
        let (title, body, collected_at) = match frontmatter::parse(&content) {
            Ok((mapping, body)) => {
                let title = mapping
                    .get("title")
                    .and_then(|v| v.as_str())
                    .filter(|t| !t.trim().is_empty())
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| entry.title.clone());
                let collected_at = mapping
                    .get("collected_at")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                (title, body, collected_at)
            }
            Err(_) => {
                // If frontmatter is invalid, use the raw content as body
                (entry.title.clone(), content.clone(), None)
            }
        };

        scored.push(ScoredLeaf {
            file: entry.file.clone(),
            title,
            body,
            score,
            collected_at,
            index_position,
        });
    }

    let total_results = scored.len();

    // Sort
    if options.recent {
        sort_recent(&mut scored);
    } else {
        sort_relevance(&mut scored);
    }

    // Paginate
    let total_pages = if total_results == 0 {
        0
    } else {
        total_results.div_ceil(PAGE_SIZE)
    };

    let start = (options.page - 1) * PAGE_SIZE;
    let hits: Vec<SearchHit> = scored
        .into_iter()
        .skip(start)
        .take(PAGE_SIZE)
        .map(|leaf| SearchHit {
            snippet: extract_snippet(&leaf.body, &query.terms, SNIPPET_RADIUS),
            file: leaf.file,
            title: leaf.title,
            score: leaf.score,
            collected_at: leaf.collected_at,
        })
        .collect();

    Ok(SearchResult {
        hits,
        total_results,
        page: options.page,
        total_pages,
    })
}

fn sort_relevance(scored: &mut [ScoredLeaf]) {
    scored.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.index_position.cmp(&b.index_position))
    });
}

fn sort_recent(scored: &mut [ScoredLeaf]) {
    scored.sort_by(|a, b| {
        let a_date = parse_date(a.collected_at.as_deref());
        let b_date = parse_date(b.collected_at.as_deref());
        match (a_date, b_date) {
            (Some(ad), Some(bd)) => bd
                .cmp(&ad)
                .then_with(|| a.index_position.cmp(&b.index_position)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => a.index_position.cmp(&b.index_position),
        }
    });
}

fn parse_date(s: Option<&str>) -> Option<DateTime<FixedOffset>> {
    s.and_then(|v| DateTime::parse_from_rfc3339(v).ok())
}

// ── rendering ────────────────────────────────────────────────────────────────

pub fn render_human(result: &SearchResult) -> String {
    if result.total_results == 0 {
        return "no results\n".to_string();
    }

    if result.hits.is_empty() {
        return format!(
            "no results on page {} ({} total results, {} pages)\n",
            result.page, result.total_results, result.total_pages
        );
    }

    let mut output = String::new();
    for hit in &result.hits {
        output.push_str(&hit.title);
        output.push('\n');
        output.push_str("  ");
        output.push_str(&hit.snippet);
        output.push('\n');
        output.push('\n');
    }
    output.push_str(&format!(
        "page {}/{} ({} results)\n",
        result.page, result.total_pages, result.total_results
    ));
    output
}

pub fn render_json(result: &SearchResult) -> Result<String, SearchError> {
    serde_json::to_string_pretty(result).map_err(SearchError::from)
}

#[cfg(test)]
#[path = "../tests/cli_search_tests.rs"]
mod tests;
