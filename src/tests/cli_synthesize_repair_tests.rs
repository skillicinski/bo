use super::*;
use crate::domain::slug::Slug;
use crate::domain::state::{TreeMetadata, TreeState};
use crate::domain::{Branch, Leaf, Timestamp, Title, Url};
use crate::engine::config::SeededConfig;
use std::collections::HashSet;
use std::path::Path;
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

fn seeded_config(dir: &Path) -> SeededConfig {
    let tree_cfg = crate::domain::tree::TreeConfig {
        path: dir.to_path_buf(),
        name: "test-tree".to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
    };
    SeededConfig::new(crate::engine::config::Config::default(), tree_cfg)
}

fn write_state(dir: &Path, state: &TreeState) {
    let tree = crate::domain::tree::Tree::from_config(&crate::domain::tree::TreeConfig {
        path: dir.to_path_buf(),
        name: "test-tree".to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
    });
    crate::engine::state::write(&crate::domain::tree::state_path(&tree.path), state).unwrap();
}

fn read_state(dir: &Path) -> TreeState {
    let state_path = crate::domain::tree::state_path(dir);
    crate::engine::state::read(&state_path).unwrap()
}

fn leaf_record(slug: &str, file: &str, title: &str, collected_at: &str) -> Leaf {
    Leaf {
        slug: Slug::generate(slug, ""),
        file: file.to_string(),
        title: Title::parse(title).ok(),
        url: Url::parse("https://example.com").unwrap(),
        collected_at: Timestamp::parse(collected_at).unwrap(),
        summary: Some("summary text".to_string()),
    }
}

fn fresh_state(name: &str, created_at: &str, last_synthesized_at: Option<&str>) -> TreeState {
    TreeState {
        tree: TreeMetadata {
            name: name.to_string(),
            created_at: Timestamp::parse(created_at).unwrap(),
            last_synthesized_at: last_synthesized_at.map(|s| Timestamp::parse(s).unwrap()),
        },
        leaves: Vec::new(),
        branches: Vec::new(),
    }
}

fn write_leaf(dir: &Path, file: &str, content: &str) {
    let path = dir.join(file);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
}

fn branch_record(slug: &str, title: &str, leaf_slugs: &[&str]) -> Branch {
    Branch {
        slug: Slug::generate(slug, ""),
        file: format!("branch/{}.md", slug),
        title: Title::parse(title).unwrap(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        leaves: leaf_slugs.iter().map(|s| Slug::generate(s, "")).collect(),
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn missing_unbranched_new_leaf_is_pruned_not_error() {
    let dir = TempDir::new().unwrap();
    let mut state = fresh_state("test", "2026-01-01T00:00:00Z", Some("2026-02-01T00:00:00Z"));
    state.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-03-01T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-03-01T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody\n");
    write_state(dir.path(), &state);

    let cfg = seeded_config(dir.path());
    let notifications = repair_stale_branches(&cfg, &state)
        .expect("repair should succeed")
        .notifications;

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 1 orphan"));

    let repaired = read_state(dir.path());
    assert_eq!(repaired.leaves.len(), 1);
    assert_eq!(repaired.leaves[0].slug.as_str(), "leaf-b");
}

#[test]
fn missing_unbranched_leaf_never_synthesized_is_pruned() {
    let dir = TempDir::new().unwrap();
    let mut state = fresh_state("test", "2026-01-01T00:00:00Z", None);
    state.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-01-15T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody\n");
    write_state(dir.path(), &state);

    let cfg = seeded_config(dir.path());
    let notifications = repair_stale_branches(&cfg, &state)
        .expect("repair should succeed")
        .notifications;

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 1 orphan"));
    assert_eq!(read_state(dir.path()).leaves.len(), 1);
}

#[test]
fn repair_with_no_missing_files_has_empty_notifications() {
    let dir = TempDir::new().unwrap();
    let mut state = fresh_state("test", "2026-01-01T00:00:00Z", None);
    state.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-a.md", "---\ntitle: Leaf A\n---\n\nbody\n");
    write_state(dir.path(), &state);

    let cfg = seeded_config(dir.path());
    let notifications = repair_stale_branches(&cfg, &state)
        .expect("repair should succeed")
        .notifications;

    assert!(notifications.is_empty());
}

#[test]
fn all_leaves_deleted_state_repaired_to_empty() {
    let dir = TempDir::new().unwrap();
    let mut state = fresh_state("test", "2026-01-01T00:00:00Z", Some("2026-02-01T00:00:00Z"));
    state.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-03-01T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-03-01T00:00:00Z",
    ));
    write_state(dir.path(), &state);

    let cfg = seeded_config(dir.path());
    let notifications = repair_stale_branches(&cfg, &state)
        .expect("repair should succeed")
        .notifications;

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 2 orphan"));
    assert_eq!(read_state(dir.path()).leaves.len(), 0);
}

#[test]
fn repair_notifications_include_branch_repair_and_removal_messages() {
    let dir = TempDir::new().unwrap();
    let mut state = fresh_state("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));

    state.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-02-01T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-02-01T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-c",
        "leaf-c.md",
        "Leaf C",
        "2026-02-01T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-d",
        "leaf-d.md",
        "Leaf D",
        "2026-02-01T00:00:00Z",
    ));

    state
        .branches
        .push(branch_record("branch-1", "Branch 1", &["leaf-a", "leaf-b"]));
    state.branches.push(branch_record(
        "branch-2",
        "Branch 2",
        &["leaf-a", "leaf-b", "leaf-c"],
    ));
    state
        .branches
        .push(branch_record("branch-3", "Branch 3", &["leaf-c", "leaf-d"]));

    write_leaf(
        dir.path(),
        "leaf-a.md",
        "---\ntitle: Leaf A\n---\n\nbody a\n",
    );
    write_leaf(
        dir.path(),
        "leaf-b.md",
        "---\ntitle: Leaf B\n---\n\nbody b\n",
    );
    write_state(dir.path(), &state);

    let cfg = seeded_config(dir.path());
    let notifications = repair_stale_branches(&cfg, &state)
        .expect("repair should succeed")
        .notifications;

    let notification_set: HashSet<&str> = notifications.iter().map(String::as_str).collect();
    assert!(
        notification_set
            .iter()
            .any(|n| n.contains("repaired 1 branch")),
        "expected 'repaired 1 branch' in notifications: {:?}",
        notifications
    );
    assert!(
        notification_set
            .iter()
            .any(|n| n.contains("removed 1 stale branch")),
        "expected 'removed 1 stale branch' in notifications: {:?}",
        notifications
    );

    let repaired = read_state(dir.path());
    assert_eq!(repaired.branches.len(), 2);
    let branch_slugs: HashSet<&str> = repaired.branches.iter().map(|b| b.slug.as_str()).collect();
    assert!(branch_slugs.contains("branch-1"));
    assert!(branch_slugs.contains("branch-2"));
    assert!(!branch_slugs.contains("branch-3"));
}

#[test]
fn repair_stale_branches_fixes_branch_frontmatter() {
    let dir = TempDir::new().unwrap();
    let mut state = fresh_state("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));

    state.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-02-01T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-02-01T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-c",
        "leaf-c.md",
        "Leaf C",
        "2026-02-01T00:00:00Z",
    ));

    state.branches.push(branch_record(
        "test-branch",
        "Test Branch",
        &["leaf-a", "leaf-b", "leaf-c"],
    ));

    write_leaf(
        dir.path(),
        "leaf-b.md",
        "---\ntitle: Leaf B\n---\n\nbody b\n",
    );
    write_leaf(
        dir.path(),
        "leaf-c.md",
        "---\ntitle: Leaf C\n---\n\nbody c\n",
    );

    std::fs::create_dir_all(dir.path().join("branch")).unwrap();
    let branch_content = "---\ntitle: Test Branch\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\nleaves:\n- leaf-a.md\n- leaf-b.md\n- leaf-c.md\n---\n\n# Test Branch\n\nBody text with reference to Leaf A\n";
    std::fs::write(dir.path().join("branch/test-branch.md"), branch_content).unwrap();

    write_state(dir.path(), &state);

    let cfg = seeded_config(dir.path());
    let notifications = repair_stale_branches(&cfg, &state)
        .expect("repair should succeed")
        .notifications;

    assert!(
        notifications
            .iter()
            .any(|n| n.contains("frontmatter repaired")),
        "expected frontmatter repair notification in: {:?}",
        notifications
    );

    let repaired = std::fs::read_to_string(dir.path().join("branch/test-branch.md")).unwrap();
    assert!(repaired.contains("- leaf-b.md"));
    assert!(repaired.contains("- leaf-c.md"));
    assert!(!repaired.contains("- leaf-a.md"));

    assert!(repaired.contains("reference to Leaf A"));

    let repaired_state = read_state(dir.path());
    let branch = repaired_state
        .branches
        .iter()
        .find(|b| b.slug.as_str() == "test-branch")
        .unwrap();
    assert_eq!(branch.leaves.len(), 2);
}
