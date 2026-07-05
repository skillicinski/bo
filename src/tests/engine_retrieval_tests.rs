use super::*;
use crate::domain::manifest::{Manifest, TreeMeta};
use crate::domain::slug::Slug;
use crate::domain::timestamp::Timestamp;
use crate::domain::Leaf;
use std::fs;
use tempfile::TempDir;

fn make_leaf(slug: &str, title: &str, url: &str, summary: Option<&str>) -> Leaf {
    Leaf {
        slug: Slug::parse(slug).unwrap(),
        file: format!("{}.md", slug),
        title: title.to_string(),
        url: (url).to_string(),
        collected_at: Timestamp::now(),
        summary: summary.map(|s| s.to_string()),
    }
}

fn make_manifest(leaves: Vec<Leaf>) -> Manifest {
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
    fs::write(dir.join(filename), content).unwrap();
}

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
        make_leaf(
            "rust-async",
            "Rust Async",
            "http://example.com/1",
            Some("async in rust"),
        ),
        make_leaf(
            "rust-types",
            "Rust Types",
            "http://example.com/2",
            Some("rust type system"),
        ),
        make_leaf(
            "unrelated",
            "Cooking",
            "http://example.com/3",
            Some("chocolate cake"),
        ),
    ];
    let manifest = make_manifest(leaves);

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
        make_leaf("valid", "Valid", "http://example.com/1", Some("valid leaf")),
        make_leaf(
            "malformed",
            "Malformed",
            "http://example.com/2",
            Some("malformed leaf"),
        ),
        make_leaf(
            "missing",
            "Missing",
            "http://example.com/3",
            Some("missing leaf"),
        ),
    ];
    let manifest = make_manifest(leaves);

    let terms = vec!["rust".to_string()];

    let results = score_corpus(dir, &manifest, &terms);
    // malformed and missing are skipped; only valid matches
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].slug, "valid");
}
