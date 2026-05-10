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

fn make_many_leaves(count: usize, term: &str) -> Vec<(&'static str, &'static str, &'static str)> {
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
