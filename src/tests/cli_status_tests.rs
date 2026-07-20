use super::*;
use crate::domain::state::{TreeMetadata, TreeState};
use crate::domain::{Branch, Leaf, Title, Url};
use crate::domain::{Slug, Timestamp};
use std::fs;
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

fn setup_tree(dir: &Path) {
    let bo_dir = dir.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    write_state(dir, &[], &[], None);
}

fn write_state(
    dir: &Path,
    leaves: &[Leaf],
    branches: &[Branch],
    last_synthesized_at: Option<&str>,
) {
    fs::create_dir_all(dir.join(".bo")).unwrap();
    let m = TreeState {
        tree: TreeMetadata {
            name: "test".to_string(),
            created_at: Timestamp::parse("2026-05-14T09:00:00Z").unwrap(),
            last_synthesized_at: last_synthesized_at.map(|s| Timestamp::parse(s).unwrap()),
        },
        leaves: leaves.to_vec(),
        branches: branches.to_vec(),
    };
    crate::engine::state::write(&dir.join(".bo/state.json"), &m).unwrap();
}

fn leaf(slug: &str, url: &str, collected_at: &str) -> Leaf {
    Leaf {
        slug: Slug::parse(slug).unwrap(),
        file: format!("leaf/{}.md", slug),
        title: Title::parse(slug).ok(),
        url: Url::parse(url).unwrap(),
        collected_at: Timestamp::parse(collected_at).unwrap(),
        summary: None,
    }
}

fn branch_record(slug: &str, ts: &str, leaf_slugs: &[&str]) -> Branch {
    Branch {
        slug: Slug::parse(slug).unwrap(),
        file: format!("branch/{}.md", slug),
        title: Title::parse(slug).unwrap(),
        created_at: Timestamp::parse(ts).unwrap(),
        updated_at: Timestamp::parse(ts).unwrap(),
        leaves: leaf_slugs.iter().map(|s| Slug::parse(s).unwrap()).collect(),
    }
}

fn write_leaf_file(dir: &Path, filename: &str, url: &str) {
    let content = format!(
        "---\ntitle: \"{}\"\nurl: {}\ncollected_at: 2026-05-14T10:00:00Z\n---\n\n# Test\n\nBody content here.\n",
        filename.trim_end_matches(".md"),
        url
    );
    let leaf_dir = dir.join("leaf");
    fs::create_dir_all(&leaf_dir).unwrap();
    fs::write(leaf_dir.join(filename), content).unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn empty_tree_reports_zero_leaves() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    let result = compute_status(dir.path(), "test-tree", None).unwrap();

    assert_eq!(result.leaves.total, 0);
    assert_eq!(result.leaves.unsynthesized, 0);
    assert_eq!(result.branches.total, 0);
    assert!(result.branches.last_synthesized_at.is_none());
    assert_eq!(result.size.bytes, 0);
    assert!(result.hints.iter().any(|h| h.contains("bo collect")));
}

#[test]
fn unsynthesized_leaves_detected() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    write_state(
        dir.path(),
        &[
            leaf("a", "https://a.com", "2026-05-14T10:00:00Z"),
            leaf("b", "https://b.com", "2026-05-14T10:00:00Z"),
            leaf("c", "https://c.com", "2026-05-14T10:00:00Z"),
        ],
        &[],
        None, // never synthesized → all unsynthesized
    );
    write_leaf_file(dir.path(), "a.md", "https://a.com");
    write_leaf_file(dir.path(), "b.md", "https://b.com");
    write_leaf_file(dir.path(), "c.md", "https://c.com");

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert_eq!(result.leaves.total, 3);
    assert_eq!(result.leaves.unsynthesized, 3);
    assert_eq!(result.leaves.unsynthesized_slugs, vec!["a", "b", "c"]);
    assert!(result.hints.iter().any(|h| h.contains("synthesize")));
}

#[test]
fn synthesized_leaves_not_flagged() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    // 'a' was collected before last_synthesized_at → synthesized.
    // 'b' was collected after  → unsynthesized.
    write_state(
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
    assert_eq!(result.leaves.unsynthesized, 1);
    assert_eq!(result.leaves.unsynthesized_slugs, vec!["b"]);
}

#[test]
fn branch_count_and_last_synthesized() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_state(
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
        result.branches.last_synthesized_at.as_deref(),
        Some("2026-05-14T20:00:00.000Z")
    );
}

#[test]
fn missing_leaf_file_detected() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    write_state(
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

    assert_eq!(result.health.missing_leaf_files.len(), 1);
    assert_eq!(result.health.missing_leaf_files[0].file, "leaf/gone.md");
    assert!(result.hints.iter().any(|h| h.contains("missing files")));
}

#[test]
fn untracked_leaf_file_detected() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_state(dir.path(), &[], &[], None);

    // Create a leaf file that's not in the state
    write_leaf_file(dir.path(), "orphan-leaf.md", "https://orphan.com");

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert_eq!(result.health.untracked_leaf_files.len(), 1);
    assert_eq!(result.health.untracked_leaf_files[0], "orphan-leaf.md");
    assert!(result.hints.iter().any(|h| h.contains("untracked")));
}

#[test]
fn non_leaf_md_not_flagged_as_missing() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_state(dir.path(), &[], &[], None);

    // Create a non-leaf .md file (no url: in frontmatter)
    fs::write(
        dir.path().join("README.md"),
        "# My Tree\n\nJust a readme.\n",
    )
    .unwrap();

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert!(result.health.untracked_leaf_files.is_empty());
}

#[test]
fn size_computed_correctly() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());
    write_state(dir.path(), &[], &[], None);

    let content = "x".repeat(400);
    let leaf_dir = dir.path().join("leaf");
    fs::create_dir_all(&leaf_dir).unwrap();
    fs::write(leaf_dir.join("test.md"), &content).unwrap();

    let result = compute_status(dir.path(), "test", None).unwrap();

    assert_eq!(result.size.bytes, 400);
    assert_eq!(result.size.estimated_tokens, 100);
}

#[test]
fn single_unsynthesized_leaf_produces_correct_result() {
    let dir = TempDir::new().unwrap();
    setup_tree(dir.path());

    write_state(
        dir.path(),
        &[leaf("a", "https://a.com", "2026-05-14T10:00:00Z")],
        &[],
        None,
    );
    write_leaf_file(dir.path(), "a.md", "https://a.com");

    let result = compute_status(dir.path(), "my-research", None).unwrap();

    assert_eq!(result.tree_name, "my-research");
    assert_eq!(result.leaves.total, 1);
    assert_eq!(result.leaves.unsynthesized, 1);
    assert_eq!(result.leaves.unsynthesized_slugs, vec!["a"]);
    assert_eq!(result.branches.total, 0);
    assert!(result.branches.last_synthesized_at.is_none());
    assert!(result.size.bytes > 0);
    assert!(result.hints.iter().any(|h| h.contains("synthesize")));
}
