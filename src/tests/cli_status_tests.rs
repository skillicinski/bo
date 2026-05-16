use super::*;
use crate::domain::index::IndexEntry;
use crate::engine::state::{self, TreeState};
use std::collections::HashMap;
use std::fs;
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

fn setup_tree(dir: &Path) {
    let bo_dir = dir.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
}

fn write_index(dir: &Path, entries: &[IndexEntry]) {
    let index_path = dir.join(".bo/index.jsonl");
    let lines: Vec<String> = entries
        .iter()
        .map(|e| serde_json::to_string(e).unwrap())
        .collect();
    fs::write(index_path, lines.join("\n") + "\n").unwrap();
}

fn write_leaf(dir: &Path, filename: &str, url: &str) {
    let content = format!(
        "---\ntitle: \"{}\"\nurl: {}\ncollected_at: 2026-05-14T10:00:00Z\nupdated_at: 2026-05-14T10:00:00Z\n---\n\n# Test\n\nBody content here.\n",
        filename.trim_end_matches(".md"),
        url
    );
    fs::write(dir.join(filename), content).unwrap();
}

fn write_branch(dir: &Path, slug: &str, compiled_at: &str) {
    let branches_dir = dir.join("branches");
    fs::create_dir_all(&branches_dir).unwrap();
    let content = format!(
        "---\ntitle: \"{}\"\ncompiled_at: {}\nupdated_at: {}\nleaves:\n  - some-leaf\n---\n\n# {}\n\nBranch body.\n",
        slug, compiled_at, compiled_at, slug
    );
    fs::write(branches_dir.join(format!("{}.md", slug)), content).unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn empty_tree_reports_zero_leaves() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    let result = compute_status(dir.path(), "test-tree").unwrap();

    assert_eq!(result.leaves.total, 0);
    assert_eq!(result.leaves.uncompiled, 0);
    assert_eq!(result.branches.total, 0);
    assert!(result.branches.last_compiled_at.is_none());
    assert_eq!(result.size.bytes, 0);
    assert!(result.hints.iter().any(|h| h.contains("bo collect")));
}

#[test]
fn uncompiled_leaves_detected() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    let entries = vec![
        IndexEntry {
            file: "a.md".to_string(),
            title: "A".to_string(),
            url: "https://a.com".to_string(),
        },
        IndexEntry {
            file: "b.md".to_string(),
            title: "B".to_string(),
            url: "https://b.com".to_string(),
        },
        IndexEntry {
            file: "c.md".to_string(),
            title: "C".to_string(),
            url: "https://c.com".to_string(),
        },
    ];
    write_index(dir.path(), &entries);
    write_leaf(dir.path(), "a.md", "https://a.com");
    write_leaf(dir.path(), "b.md", "https://b.com");
    write_leaf(dir.path(), "c.md", "https://c.com");

    let result = compute_status(dir.path(), "test").unwrap();

    assert_eq!(result.leaves.total, 3);
    assert_eq!(result.leaves.uncompiled, 3);
    assert_eq!(result.leaves.uncompiled_slugs, vec!["a", "b", "c"]);
    assert!(result.hints.iter().any(|h| h.contains("compile")));
}

#[test]
fn compiled_leaves_not_flagged() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    let entries = vec![
        IndexEntry {
            file: "a.md".to_string(),
            title: "A".to_string(),
            url: "https://a.com".to_string(),
        },
        IndexEntry {
            file: "b.md".to_string(),
            title: "B".to_string(),
            url: "https://b.com".to_string(),
        },
    ];
    write_index(dir.path(), &entries);
    write_leaf(dir.path(), "a.md", "https://a.com");
    write_leaf(dir.path(), "b.md", "https://b.com");

    // Mark 'a' as compiled in state
    let mut compiled = HashMap::new();
    compiled.insert("a".to_string(), "2026-05-14T10:00:00Z".to_string());
    let tree_state = TreeState {
        compiled_leaves: compiled,
    };
    state::write_state(&dir.path().join(".bo/state.json"), &tree_state).unwrap();

    let result = compute_status(dir.path(), "test").unwrap();

    assert_eq!(result.leaves.total, 2);
    assert_eq!(result.leaves.uncompiled, 1);
    assert_eq!(result.leaves.uncompiled_slugs, vec!["b"]);
}

#[test]
fn branch_count_and_last_compiled() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_index(dir.path(), &[]);

    write_branch(dir.path(), "branch-one", "2026-05-13T10:00:00Z");
    write_branch(dir.path(), "branch-two", "2026-05-14T20:00:00Z");

    let result = compute_status(dir.path(), "test").unwrap();

    assert_eq!(result.branches.total, 2);
    assert_eq!(
        result.branches.last_compiled_at.as_deref(),
        Some("2026-05-14T20:00:00Z")
    );
}

#[test]
fn orphan_index_entry_detected() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    let entries = vec![
        IndexEntry {
            file: "exists.md".to_string(),
            title: "Exists".to_string(),
            url: "https://e.com".to_string(),
        },
        IndexEntry {
            file: "gone.md".to_string(),
            title: "Gone".to_string(),
            url: "https://g.com".to_string(),
        },
    ];
    write_index(dir.path(), &entries);
    write_leaf(dir.path(), "exists.md", "https://e.com");
    // Don't create gone.md

    let result = compute_status(dir.path(), "test").unwrap();

    assert_eq!(result.health.orphan_index_entries.len(), 1);
    assert_eq!(result.health.orphan_index_entries[0].file, "gone.md");
    assert!(result.hints.iter().any(|h| h.contains("missing files")));
}

#[test]
fn missing_from_index_detected() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_index(dir.path(), &[]);

    // Create a leaf file that's not in the index
    write_leaf(dir.path(), "orphan-leaf.md", "https://orphan.com");

    let result = compute_status(dir.path(), "test").unwrap();

    assert_eq!(result.health.missing_from_index.len(), 1);
    assert_eq!(result.health.missing_from_index[0], "orphan-leaf.md");
    assert!(result.hints.iter().any(|h| h.contains("not indexed")));
}

#[test]
fn non_leaf_md_not_flagged_as_missing() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_index(dir.path(), &[]);

    // Create a non-leaf .md file (no url: in frontmatter)
    fs::write(
        dir.path().join("README.md"),
        "# My Tree\n\nJust a readme.\n",
    )
    .unwrap();

    let result = compute_status(dir.path(), "test").unwrap();

    assert!(result.health.missing_from_index.is_empty());
}

#[test]
fn size_computed_correctly() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_index(dir.path(), &[]);

    // Write a known-size leaf
    let content = "x".repeat(400);
    fs::write(dir.path().join("test.md"), &content).unwrap();

    let result = compute_status(dir.path(), "test").unwrap();

    assert_eq!(result.size.bytes, 400);
    assert_eq!(result.size.estimated_tokens, 100);
}

#[test]
fn single_uncompiled_leaf_produces_correct_result() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    let entries = vec![IndexEntry {
        file: "a.md".to_string(),
        title: "A".to_string(),
        url: "https://a.com".to_string(),
    }];
    write_index(dir.path(), &entries);
    write_leaf(dir.path(), "a.md", "https://a.com");

    let result = compute_status(dir.path(), "my-research").unwrap();

    assert_eq!(result.tree_name, "my-research");
    assert_eq!(result.leaves.total, 1);
    assert_eq!(result.leaves.uncompiled, 1);
    assert_eq!(result.leaves.uncompiled_slugs, vec!["a"]);
    assert_eq!(result.branches.total, 0);
    assert!(result.branches.last_compiled_at.is_none());
    assert!(result.size.bytes > 0);
    assert!(result.hints.iter().any(|h| h.contains("compile")));
}
