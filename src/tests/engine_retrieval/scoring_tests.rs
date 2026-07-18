// score_corpus unit tests: IDF scoring, token-level matching, density ranking.

use crate::domain::manifest::{Manifest, TreeMeta};
use crate::domain::slug::Slug;
use crate::domain::timestamp::Timestamp;
use crate::domain::{Leaf, Title, Url};
use crate::engine::retrieval::score_corpus;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ── in-memory record builders (for score_corpus) ─────

fn leaf_record(slug: &str, title: &str, url: &str, summary: Option<&str>) -> Leaf {
    Leaf {
        slug: Slug::parse(slug).unwrap(),
        file: format!("{}.md", slug),
        title: Title::parse(title).ok(),
        url: Url::parse(url).unwrap(),
        collected_at: Timestamp::now(),
        summary: summary.map(|s| s.to_string()),
    }
}

fn manifest_record(leaves: Vec<Leaf>) -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: "test".to_string(),
            created_at: Timestamp::now(),
            last_compiled_at: None,
        },
        leaves,
        branches: Vec::new(),
    }
}

fn write_leaf(dir: &Path, filename: &str, title: &str, body: &str) {
    let content = format!("---\ntitle: {}\n---\n{}", title, body);
    fs::write(dir.join(filename), content).unwrap()
}

// ── corpus scoring tests ─────────────────────────────────────────────

#[test]
fn any_term_counts_returns_partial_matches() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    write_leaf(
        dir,
        "rust-async.md",
        "Rust Async",
        "async await tokio runtime",
    );
    write_leaf(
        dir,
        "rust-types.md",
        "Rust Types",
        "rust type system generics",
    );
    write_leaf(dir, "unrelated.md", "Cooking", "recipe for chocolate cake");

    let leaves = vec![
        leaf_record(
            "rust-async",
            "Rust Async",
            "http://example.com/1",
            Some("async in rust"),
        ),
        leaf_record(
            "rust-types",
            "Rust Types",
            "http://example.com/2",
            Some("rust type system"),
        ),
        leaf_record(
            "unrelated",
            "Cooking",
            "http://example.com/3",
            Some("chocolate cake"),
        ),
    ];
    let manifest = manifest_record(leaves);

    let terms = vec!["rust".to_string(), "async".to_string()];
    let results = score_corpus(dir, &manifest, &terms);

    // Both rust leaves match (OR semantics); cooking does not
    assert_eq!(results.len(), 2);
    let slugs: Vec<&str> = results.iter().map(|r| r.slug.as_str()).collect();
    assert!(slugs.contains(&"rust-async"));
    assert!(slugs.contains(&"rust-types"));
}

#[test]
fn missing_and_malformed_files_are_skipped() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Write one valid leaf
    write_leaf(
        dir,
        "valid.md",
        "Valid",
        "some content about rust programming",
    );
    // Write one malformed leaf (no frontmatter delimiter)
    fs::write(
        dir.join("malformed.md"),
        "no frontmatter here just rust text",
    )
    .unwrap();
    // "missing.md" is never written

    let leaves = vec![
        leaf_record("valid", "Valid", "http://example.com/1", Some("valid leaf")),
        leaf_record(
            "malformed",
            "Malformed",
            "http://example.com/2",
            Some("malformed leaf"),
        ),
        leaf_record(
            "missing",
            "Missing",
            "http://example.com/3",
            Some("missing leaf"),
        ),
    ];
    let manifest = manifest_record(leaves);

    let terms = vec!["rust".to_string()];

    let results = score_corpus(dir, &manifest, &terms);
    // malformed and missing are skipped; only valid matches
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].slug, "valid");
}

// ── token+IDF scorer tests ──────────────────────────────────────────

#[test]
fn scorer_token_matching_does_not_match_substrings() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    write_leaf(dir, "trust.md", "Trust", "trust building exercise");
    write_leaf(dir, "rust.md", "Rust", "rust programming language");
    write_leaf(dir, "cooking.md", "Cooking", "recipe");

    let leaves = vec![
        leaf_record("trust", "Trust", "http://x.com/1", Some("trust")),
        leaf_record("rust", "Rust", "http://x.com/2", Some("rust")),
        leaf_record("cooking", "Cooking", "http://x.com/3", Some("cooking")),
    ];
    let manifest = manifest_record(leaves);

    let terms = vec!["rust".to_string()];
    let results = score_corpus(dir, &manifest, &terms);

    // only rust.md should match; trust.md must NOT match
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].slug, "rust");
}

#[test]
fn scorer_idf_rare_term_outranks_common_term_at_equal_hits() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // "async" appears in one doc; "rust" appears in many
    write_leaf(dir, "async-only.md", "Async", "async runtime");
    write_leaf(dir, "rust-a.md", "Rust A", "rust basics");
    write_leaf(dir, "rust-b.md", "Rust B", "rust advanced");
    write_leaf(dir, "rust-c.md", "Rust C", "rust ecosystem");
    write_leaf(dir, "rust-d.md", "Rust D", "rust tooling");

    let leaves = vec![
        leaf_record("async-only", "Async", "http://x.com/1", Some("async")),
        leaf_record("rust-a", "Rust A", "http://x.com/2", Some("rust")),
        leaf_record("rust-b", "Rust B", "http://x.com/3", Some("rust")),
        leaf_record("rust-c", "Rust C", "http://x.com/4", Some("rust")),
        leaf_record("rust-d", "Rust D", "http://x.com/5", Some("rust")),
    ];
    let manifest = manifest_record(leaves);

    let terms = vec!["async".to_string(), "rust".to_string()];
    let results = score_corpus(dir, &manifest, &terms);

    assert!(!results.is_empty());
    assert_eq!(results[0].slug, "async-only");
}

#[test]
fn scorer_short_focused_doc_outranks_long_sparse_doc() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path();

    // Short doc: "rust" appears in 2 of 3 tokens
    write_leaf(dir, "short.md", "Rust", "rust is safe");
    // Long doc: "rust" appears once in many tokens
    let filler = "filler ".repeat(100);
    let long_body = format!("rust {}", filler);
    write_leaf(dir, "long.md", "Rust Long", &long_body);

    let leaves = vec![
        leaf_record("short", "Rust", "http://x.com/1", Some("short")),
        leaf_record("long", "Rust Long", "http://x.com/2", Some("long")),
    ];
    let manifest = manifest_record(leaves);

    let terms = vec!["rust".to_string()];
    let results = score_corpus(dir, &manifest, &terms);

    // Short doc has higher density → higher normalized score
    assert_eq!(results.len(), 2);
    assert!(results[0].score > results[1].score);
    assert_eq!(results[0].slug, "short");
}
