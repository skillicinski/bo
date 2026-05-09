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

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── core matching tests ──────────────────────────────────────────────────

    #[test]
    fn matches_single_term() {
        assert!(matches_all_terms("hello world", &[s("hello")]));
        assert!(matches_all_terms("hello world", &[s("world")]));
        assert!(!matches_all_terms("hello world", &[s("missing")]));
    }

    #[test]
    fn matches_multiple_terms_and_semantics() {
        let terms = vec![s("rust"), s("ownership")];
        assert!(matches_all_terms("rust has ownership semantics", &terms));
        assert!(!matches_all_terms("rust is fast", &terms));
        assert!(!matches_all_terms("ownership is important", &terms));
    }

    #[test]
    fn matches_phrase_as_single_arg() {
        let terms = vec![s("borrow checker")];
        assert!(matches_all_terms(
            "the borrow checker ensures safety",
            &terms
        ));
        assert!(!matches_all_terms("borrow and checker separately", &terms));
    }

    #[test]
    fn matches_requires_pre_lowered_input() {
        // The function does NOT lowercase internally — caller must.
        // "Hello" (not lowered) won't be found in lowered content.
        assert!(!matches_all_terms("hello world", &["Hello".to_string()]));
        // But properly lowered input works:
        assert!(matches_all_terms("hello world", &["hello".to_string()]));
    }

    #[test]
    fn score_empty_content_returns_zero() {
        assert_eq!(score_relevance("", &[s("anything")]), 0);
    }

    #[test]
    fn score_single_occurrence() {
        // "rust" in "rust is great" (3 words) → 1 * 1000 / 3 = 333
        assert_eq!(score_relevance("rust is great", &[s("rust")]), 333);
    }

    #[test]
    fn score_multiple_occurrences_density() {
        // "rust" appears 3 times in "rust rust rust xyzab" (4 words) → 3 * 1000 / 4 = 750
        let content = "rust rust rust xyzab";
        assert_eq!(score_relevance(content, &[s("rust")]), 750);
    }

    #[test]
    fn score_multiple_terms_summed() {
        // "ab" appears 2x, "cd" appears 1x in "ab cd ab xy" (4 words)
        // total = 3, score = 3000/4 = 750
        assert_eq!(score_relevance("ab cd ab xy", &[s("ab"), s("cd")]), 750);
    }

    #[test]
    fn score_short_doc_beats_long_doc_at_same_count() {
        let short = "rust is great"; // 3 words, 1 occurrence → 333
        let long = "rust is a systems programming language that is great for many things"; // 12 words, 1 occurrence → 83
        assert!(score_relevance(short, &[s("rust")]) > score_relevance(long, &[s("rust")]));
    }

    #[test]
    fn overlapping_matches_counted() {
        // str::matches is non-overlapping: "aaa".matches("aa") = 1
        // "aaa" is 1 word, score = 1 * 1000 / 1 = 1000
        assert_eq!(score_relevance("aaa", &[s("aa")]), 1000);
    }

    // ── snippet extraction tests ─────────────────────────────────────────────

    #[test]
    fn snippet_match_in_middle() {
        let body = "a".repeat(100) + "TARGET" + &"b".repeat(100);
        let snippet = extract_snippet(&body, &[s("target")], 5);
        assert!(snippet.starts_with('…'));
        assert!(snippet.ends_with('…'));
        assert!(snippet.contains("TARGET"));
    }

    #[test]
    fn snippet_match_at_start() {
        let body = "TARGET".to_string() + &"x".repeat(200);
        let snippet = extract_snippet(&body, &[s("target")], 5);
        assert!(!snippet.starts_with('…'));
        assert!(snippet.ends_with('…'));
        assert!(snippet.contains("TARGET"));
    }

    #[test]
    fn snippet_match_at_end() {
        let body = "x".repeat(200) + "TARGET";
        let snippet = extract_snippet(&body, &[s("target")], 5);
        assert!(snippet.starts_with('…'));
        assert!(!snippet.ends_with('…'));
        assert!(snippet.contains("TARGET"));
    }

    #[test]
    fn snippet_body_shorter_than_radius() {
        let body = "short TARGET text";
        let snippet = extract_snippet(body, &[s("target")], 80);
        assert!(!snippet.starts_with('…'));
        assert!(!snippet.ends_with('…'));
        assert_eq!(snippet, "short TARGET text");
    }

    #[test]
    fn snippet_fallback_when_no_match_in_body() {
        let body = "This is the body content without the search term.";
        let snippet = extract_snippet(body, &[s("nonexistent")], 80);
        assert_eq!(snippet, body);
    }

    #[test]
    fn snippet_fallback_truncates_long_body() {
        let body = "x".repeat(300);
        let snippet = extract_snippet(&body, &[s("nonexistent")], 80);
        assert_eq!(snippet.len(), FALLBACK_SNIPPET_LEN + "…".len());
        assert!(snippet.ends_with('…'));
    }

    #[test]
    fn snippet_empty_body() {
        let snippet = extract_snippet("", &[s("term")], 80);
        assert_eq!(snippet, "");
    }

    #[test]
    fn snippet_newlines_collapsed_to_space() {
        let body = "before\n\nTARGET\n\nafter";
        let snippet = extract_snippet(body, &[s("target")], 80);
        assert!(!snippet.contains('\n'));
        assert!(snippet.contains("before TARGET after"));
    }

    #[test]
    fn snippet_multibyte_utf8_safe() {
        // Japanese characters are 3 bytes each
        let body = "あ".repeat(50) + "TARGET" + &"い".repeat(50);
        let snippet = extract_snippet(&body, &[s("target")], 5);
        assert!(snippet.contains("TARGET"));
        // Should not panic or produce invalid UTF-8
        assert!(snippet.is_char_boundary(0));
    }

    // ── orchestration tests ──────────────────────────────────────────────────

    #[test]
    fn search_basic_single_term() {
        let dir = setup_tree(&[
            (
                "match.md",
                "Matching Leaf",
                "This document talks about rust programming.",
            ),
            (
                "nomatch.md",
                "Other Leaf",
                "This document talks about python.",
            ),
        ]);

        let result = search_leaves(
            dir.path(),
            &query(&["rust"]),
            &SearchOptions {
                page: 1,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.total_results, 1);
        assert_eq!(result.hits[0].file, "match.md");
    }

    #[test]
    fn search_and_semantics() {
        let dir = setup_tree(&[
            ("both.md", "Both", "rust ownership is key to safety."),
            ("only-rust.md", "Only Rust", "rust is fast."),
            ("only-own.md", "Only Own", "ownership matters in c++."),
        ]);

        let result = search_leaves(
            dir.path(),
            &query(&["rust", "ownership"]),
            &SearchOptions {
                page: 1,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.total_results, 1);
        assert_eq!(result.hits[0].file, "both.md");
    }

    #[test]
    fn search_phrase_matching() {
        let dir = setup_tree(&[
            ("phrase.md", "Phrase", "the borrow checker is great."),
            ("separate.md", "Separate", "borrow and then checker later."),
        ]);

        let result = search_leaves(
            dir.path(),
            &query(&["borrow checker"]),
            &SearchOptions {
                page: 1,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.total_results, 1);
        assert_eq!(result.hits[0].file, "phrase.md");
    }

    #[test]
    fn search_case_insensitive() {
        let dir = setup_tree(&[(
            "upper.md",
            "Upper",
            "RUST is GREAT for Systems Programming.",
        )]);

        let result = search_leaves(
            dir.path(),
            &query(&["rust", "systems"]),
            &SearchOptions {
                page: 1,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.total_results, 1);
    }

    #[test]
    fn search_relevance_ordering() {
        let dir = setup_tree(&[
            ("low.md", "Low", &format!("rust. {}", "x".repeat(500))),
            ("high.md", "High", "rust rust rust in short doc."),
        ]);

        let result = search_leaves(
            dir.path(),
            &query(&["rust"]),
            &SearchOptions {
                page: 1,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.hits[0].file, "high.md");
        assert_eq!(result.hits[1].file, "low.md");
    }

    #[test]
    fn search_recent_ordering() {
        let dir = setup_tree_with_dates(&[
            ("old.md", "Old", "rust programming", "2025-01-01T00:00:00Z"),
            ("new.md", "New", "rust programming", "2025-06-01T00:00:00Z"),
            ("mid.md", "Mid", "rust programming", "2025-03-01T00:00:00Z"),
        ]);

        let result = search_leaves(
            dir.path(),
            &query(&["rust"]),
            &SearchOptions {
                page: 1,
                recent: true,
            },
        )
        .unwrap();

        assert_eq!(result.hits[0].file, "new.md");
        assert_eq!(result.hits[1].file, "mid.md");
        assert_eq!(result.hits[2].file, "old.md");
    }

    #[test]
    fn search_pagination_page_1() {
        let dir = setup_tree(&make_many_leaves(7, "rust"));

        let result = search_leaves(
            dir.path(),
            &query(&["rust"]),
            &SearchOptions {
                page: 1,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.total_results, 7);
        assert_eq!(result.hits.len(), 5);
        assert_eq!(result.page, 1);
        assert_eq!(result.total_pages, 2);
    }

    #[test]
    fn search_pagination_page_2() {
        let dir = setup_tree(&make_many_leaves(7, "rust"));

        let result = search_leaves(
            dir.path(),
            &query(&["rust"]),
            &SearchOptions {
                page: 2,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.hits.len(), 2);
        assert_eq!(result.page, 2);
        assert_eq!(result.total_pages, 2);
    }

    #[test]
    fn search_pagination_out_of_range() {
        let dir = setup_tree(&make_many_leaves(3, "rust"));

        let result = search_leaves(
            dir.path(),
            &query(&["rust"]),
            &SearchOptions {
                page: 5,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.total_results, 3);
        assert_eq!(result.hits.len(), 0);
        assert_eq!(result.total_pages, 1);
    }

    #[test]
    fn search_no_results() {
        let dir = setup_tree(&[("only.md", "Only", "nothing relevant here.")]);

        let result = search_leaves(
            dir.path(),
            &query(&["nonexistent"]),
            &SearchOptions {
                page: 1,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.total_results, 0);
        assert_eq!(result.hits.len(), 0);
        assert_eq!(result.total_pages, 0);
    }

    #[test]
    fn search_skips_missing_files() {
        let dir = TempDir::new().unwrap();
        // Write index referencing a file that doesn't exist
        let index_content = r#"{"file":"exists.md","title":"Exists","url":"https://example.com/exists"}
{"file":"missing.md","title":"Missing","url":"https://example.com/missing"}"#;
        fs::write(dir.path().join("index.jsonl"), index_content).unwrap();
        write_leaf_file(
            dir.path(),
            "exists.md",
            "Exists",
            "rust programming",
            "2025-01-01T00:00:00Z",
        );

        let result = search_leaves(
            dir.path(),
            &query(&["rust"]),
            &SearchOptions {
                page: 1,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.total_results, 1);
        assert_eq!(result.hits[0].file, "exists.md");
    }

    #[test]
    fn search_matches_frontmatter_content() {
        let dir = setup_tree(&[(
            "example.md",
            "Example Article",
            "body without the search term",
        )]);

        // "example" appears in the title/frontmatter but not body
        let result = search_leaves(
            dir.path(),
            &query(&["example"]),
            &SearchOptions {
                page: 1,
                recent: false,
            },
        )
        .unwrap();

        assert_eq!(result.total_results, 1);
    }

    // ── rendering tests ──────────────────────────────────────────────────────

    #[test]
    fn render_human_with_results() {
        let result = SearchResult {
            hits: vec![SearchHit {
                file: "test.md".to_string(),
                title: "Test Article".to_string(),
                snippet: "…some context around match…".to_string(),
                score: 50,
                collected_at: Some("2025-01-01T00:00:00Z".to_string()),
            }],
            total_results: 1,
            page: 1,
            total_pages: 1,
        };

        let output = render_human(&result);
        assert!(output.contains("Test Article"));
        assert!(output.contains("…some context around match…"));
        assert!(output.contains("page 1/1 (1 results)"));
    }

    #[test]
    fn render_human_no_results() {
        let result = SearchResult {
            hits: vec![],
            total_results: 0,
            page: 1,
            total_pages: 0,
        };
        assert_eq!(render_human(&result), "no results\n");
    }

    #[test]
    fn render_human_empty_page() {
        let result = SearchResult {
            hits: vec![],
            total_results: 7,
            page: 3,
            total_pages: 2,
        };
        let output = render_human(&result);
        assert!(output.contains("no results on page 3"));
        assert!(output.contains("7 total results"));
    }

    #[test]
    fn render_json_valid_and_parseable() {
        let result = SearchResult {
            hits: vec![SearchHit {
                file: "test.md".to_string(),
                title: "Test".to_string(),
                snippet: "snippet text".to_string(),
                score: 42,
                collected_at: None,
            }],
            total_results: 1,
            page: 1,
            total_pages: 1,
        };

        let json = render_json(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["hits"][0]["file"], "test.md");
        assert_eq!(parsed["hits"][0]["title"], "Test");
        assert_eq!(parsed["total_results"], 1);
        assert_eq!(parsed["page"], 1);
        assert_eq!(parsed["total_pages"], 1);
        // score is intentionally hidden from JSON output
        assert!(parsed["hits"][0].get("score").is_none());
    }

    #[test]
    fn render_json_empty_results() {
        let result = SearchResult {
            hits: vec![],
            total_results: 0,
            page: 1,
            total_pages: 0,
        };

        let json = render_json(&result).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["hits"].as_array().unwrap().len(), 0);
        assert_eq!(parsed["total_results"], 0);
    }

    // ── test helpers ─────────────────────────────────────────────────────────

    fn s(val: &str) -> String {
        val.to_lowercase()
    }

    fn query(terms: &[&str]) -> SearchQuery {
        SearchQuery {
            terms: terms.iter().map(|t| t.to_lowercase()).collect(),
        }
    }

    fn setup_tree(leaves: &[(&str, &str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        let mut index_lines = Vec::new();

        for (file, title, body) in leaves {
            write_leaf_file(dir.path(), file, title, body, "2025-01-01T00:00:00Z");
            index_lines.push(format!(
                r#"{{"file":"{}","title":"{}","url":"https://example.com/{}"}}"#,
                file, title, file
            ));
        }

        fs::write(
            dir.path().join("index.jsonl"),
            index_lines.join("\n") + "\n",
        )
        .unwrap();

        dir
    }

    fn setup_tree_with_dates(leaves: &[(&str, &str, &str, &str)]) -> TempDir {
        let dir = TempDir::new().unwrap();
        let mut index_lines = Vec::new();

        for (file, title, body, date) in leaves {
            write_leaf_file(dir.path(), file, title, body, date);
            index_lines.push(format!(
                r#"{{"file":"{}","title":"{}","url":"https://example.com/{}"}}"#,
                file, title, file
            ));
        }

        fs::write(
            dir.path().join("index.jsonl"),
            index_lines.join("\n") + "\n",
        )
        .unwrap();

        dir
    }

    fn make_many_leaves(
        count: usize,
        term: &str,
    ) -> Vec<(&'static str, &'static str, &'static str)> {
        // Leak strings for test convenience — acceptable in tests
        (0..count)
            .map(|i| {
                let file: &'static str = Box::leak(format!("leaf-{}.md", i).into_boxed_str());
                let title: &'static str = Box::leak(format!("Leaf {}", i).into_boxed_str());
                let body: &'static str =
                    Box::leak(format!("content about {} number {}", term, i).into_boxed_str());
                (file, title, body)
            })
            .collect()
    }

    fn write_leaf_file(tree_dir: &Path, file: &str, title: &str, body: &str, date: &str) {
        let content = format!(
            "---\ntitle: \"{}\"\nurl: https://example.com/{}\ncollected_at: {}\nupdated_at: {}\n---\n\n# {}\n\n{}\n",
            title, file, date, date, title, body
        );
        let path = tree_dir.join(file);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }
}
