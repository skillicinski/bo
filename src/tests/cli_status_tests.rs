use super::*;
use crate::domain::manifest::{Manifest, TreeMeta};
use crate::domain::{Branch, Leaf, Title, Url};
use crate::domain::{Slug, Timestamp};
use std::fs;
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

fn setup_tree(dir: &Path) {
    let bo_dir = dir.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    write_manifest(dir, &[], &[], None);
}

fn write_manifest(
    dir: &Path,
    leaves: &[Leaf],
    branches: &[Branch],
    last_compiled_at: Option<&str>,
) {
    fs::create_dir_all(dir.join(".bo")).unwrap();
    let m = Manifest {
        tree: TreeMeta {
            name: "test".to_string(),
            created_at: Timestamp::parse("2026-05-14T09:00:00Z").unwrap(),
            last_compiled_at: last_compiled_at.map(|s| Timestamp::parse(s).unwrap()),
        },
        leaves: leaves.to_vec(),
        branches: branches.to_vec(),
    };
    crate::engine::manifest::write(&dir.join(".bo/manifest.json"), &m).unwrap();
}

fn leaf(slug: &str, url: &str, collected_at: &str) -> Leaf {
    Leaf {
        slug: Slug::parse(slug).unwrap(),
        file: format!("{}.md", slug),
        title: Title::parse(slug).ok(),
        url: Url::parse(url).unwrap(),
        collected_at: Timestamp::parse(collected_at).unwrap(),
        summary: None,
    }
}

fn branch_record(slug: &str, ts: &str, leaf_slugs: &[&str]) -> Branch {
    Branch {
        slug: Slug::parse(slug).unwrap(),
        file: format!("branches/{}.md", slug),
        title: Title::parse(slug).unwrap(),
        created_at: Timestamp::parse(ts).unwrap(),
        updated_at: Timestamp::parse(ts).unwrap(),
        leaves: leaf_slugs.iter().map(|s| Slug::parse(s).unwrap()).collect(),
    }
}

fn write_leaf_file(dir: &Path, filename: &str, url: &str) {
    let content = format!(
        "---\ntitle: \"{}\"\nurl: {}\ncollected_at: 2026-05-14T10:00:00Z\nupdated_at: 2026-05-14T10:00:00Z\n---\n\n# Test\n\nBody content here.\n",
        filename.trim_end_matches(".md"),
        url
    );
    fs::write(dir.join(filename), content).unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn empty_tree_reports_zero_leaves() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    let result = compute_status(dir.path(), "test-tree", None).unwrap();

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

    write_manifest(
        dir.path(),
        &[
            leaf("a", "https://a.com", "2026-05-14T10:00:00Z"),
            leaf("b", "https://b.com", "2026-05-14T10:00:00Z"),
            leaf("c", "https://c.com", "2026-05-14T10:00:00Z"),
        ],
        &[],
        None, // never compiled → all uncompiled
    );
    write_leaf_file(dir.path(), "a.md", "https://a.com");
    write_leaf_file(dir.path(), "b.md", "https://b.com");
    write_leaf_file(dir.path(), "c.md", "https://c.com");

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert_eq!(result.leaves.total, 3);
    assert_eq!(result.leaves.uncompiled, 3);
    assert_eq!(result.leaves.uncompiled_slugs, vec!["a", "b", "c"]);
    assert!(result.hints.iter().any(|h| h.contains("compile")));
}

#[test]
fn compiled_leaves_not_flagged() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    // 'a' was collected before last_compiled_at → compiled.
    // 'b' was collected after  → uncompiled.
    write_manifest(
        dir.path(),
        &[
            leaf("a", "https://a.com", "2026-05-14T08:00:00Z"),
            leaf("b", "https://b.com", "2026-05-14T12:00:00Z"),
        ],
        &[],
        Some("2026-05-14T10:00:00Z"),
    );
    write_leaf_file(dir.path(), "a.md", "https://a.com");
    write_leaf_file(dir.path(), "b.md", "https://b.com");

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert_eq!(result.leaves.total, 2);
    assert_eq!(result.leaves.uncompiled, 1);
    assert_eq!(result.leaves.uncompiled_slugs, vec!["b"]);
}

#[test]
fn branch_count_and_last_compiled() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_manifest(
        dir.path(),
        &[],
        &[
            branch_record("branch-one", "2026-05-13T10:00:00Z", &[]),
            branch_record("branch-two", "2026-05-14T20:00:00Z", &[]),
        ],
        Some("2026-05-14T20:00:00Z"),
    );

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert_eq!(result.branches.total, 2);
    assert_eq!(
        result.branches.last_compiled_at.as_deref(),
        Some("2026-05-14T20:00:00.000Z")
    );
}

#[test]
fn orphan_manifest_entry_detected() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    write_manifest(
        dir.path(),
        &[
            leaf("exists", "https://e.com", "2026-05-14T10:00:00Z"),
            leaf("gone", "https://g.com", "2026-05-14T10:00:00Z"),
        ],
        &[],
        None,
    );
    write_leaf_file(dir.path(), "exists.md", "https://e.com");
    // Don't create gone.md

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert_eq!(result.health.orphan_index_entries.len(), 1);
    assert_eq!(result.health.orphan_index_entries[0].file, "gone.md");
    assert!(result.hints.iter().any(|h| h.contains("missing files")));
}

#[test]
fn missing_from_index_detected() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_manifest(dir.path(), &[], &[], None);

    // Create a leaf file that's not in the manifest
    write_leaf_file(dir.path(), "orphan-leaf.md", "https://orphan.com");

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert_eq!(result.health.missing_from_index.len(), 1);
    assert_eq!(result.health.missing_from_index[0], "orphan-leaf.md");
    assert!(result.hints.iter().any(|h| h.contains("not indexed")));
}

#[test]
fn non_leaf_md_not_flagged_as_missing() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_manifest(dir.path(), &[], &[], None);

    // Create a non-leaf .md file (no url: in frontmatter)
    fs::write(
        dir.path().join("README.md"),
        "# My Tree\n\nJust a readme.\n",
    )
    .unwrap();

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert!(result.health.missing_from_index.is_empty());
}

#[test]
fn size_computed_correctly() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_manifest(dir.path(), &[], &[], None);

    let content = "x".repeat(400);
    fs::write(dir.path().join("test.md"), &content).unwrap();

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert_eq!(result.size.bytes, 400);
    assert_eq!(result.size.estimated_tokens, 100);
}

#[test]
fn single_uncompiled_leaf_produces_correct_result() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    write_manifest(
        dir.path(),
        &[leaf("a", "https://a.com", "2026-05-14T10:00:00Z")],
        &[],
        None,
    );
    write_leaf_file(dir.path(), "a.md", "https://a.com");

    let result = compute_status(dir.path(), "my-research", None).unwrap();

    assert_eq!(result.tree_name, "my-research");
    assert_eq!(result.leaves.total, 1);
    assert_eq!(result.leaves.uncompiled, 1);
    assert_eq!(result.leaves.uncompiled_slugs, vec!["a"]);
    assert_eq!(result.branches.total, 0);
    assert!(result.branches.last_compiled_at.is_none());
    assert!(result.size.bytes > 0);
    assert!(result.hints.iter().any(|h| h.contains("compile")));
}
