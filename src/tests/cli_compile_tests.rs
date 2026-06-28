use super::{plan, render_human, CompileResult};
use crate::domain::manifest::{BranchRecord, LeafRecord, Manifest, TreeMeta};
use crate::domain::{Slug, Timestamp};
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

fn write_manifest(dir: &Path, manifest: &Manifest) {
    let tree = crate::domain::tree::Tree::from_config(&crate::domain::tree::TreeConfig {
        path: dir.to_path_buf(),
        name: "test-tree".to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
    });
    crate::domain::manifest::write(&crate::domain::tree::manifest_path(&tree.path), manifest)
        .unwrap();
}

fn read_manifest(dir: &Path) -> Manifest {
    let manifest_path = crate::domain::tree::manifest_path(dir);
    crate::domain::manifest::read(&manifest_path).unwrap()
}

fn leaf_record(slug: &str, file: &str, title: &str, collected_at: &str) -> LeafRecord {
    LeafRecord {
        slug: Slug::generate(slug, ""),
        file: file.to_string(),
        title: title.to_string(),
        url: ("https://example.com").to_string(),
        collected_at: Timestamp::parse(collected_at).unwrap(),
        summary: Some("summary text".to_string()),
    }
}

fn fresh_manifest(name: &str, created_at: &str, last_compiled_at: Option<&str>) -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: name.to_string(),
            created_at: Timestamp::parse(created_at).unwrap(),
            last_compiled_at: last_compiled_at.map(|s| Timestamp::parse(s).unwrap()),
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

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn missing_unbranched_new_leaf_is_pruned_not_error() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-02-01T00:00:00Z"));
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-03-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-03-01T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody\n");
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications =
        plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 1 orphan"));

    let repaired = read_manifest(dir.path());
    assert_eq!(repaired.leaves.len(), 1);
    assert_eq!(repaired.leaves[0].slug.as_str(), "leaf-b");
}

#[test]
fn missing_unbranched_leaf_never_compiled_is_pruned() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", None);
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-01-15T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody\n");
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications =
        plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 1 orphan"));
    assert_eq!(read_manifest(dir.path()).leaves.len(), 1);
}

#[test]
fn repair_with_no_missing_files_has_empty_notifications() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", None);
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-a.md", "---\ntitle: Leaf A\n---\n\nbody\n");
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications =
        plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert!(notifications.is_empty());
}

#[test]
fn all_leaves_deleted_manifest_repaired_to_empty() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-02-01T00:00:00Z"));
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-03-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-03-01T00:00:00Z",
    ));
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications =
        plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 2 orphan"));
    assert_eq!(read_manifest(dir.path()).leaves.len(), 0);
}

#[test]
fn compile_result_notifications_serialized_and_omitted_when_empty() {
    let result = CompileResult {
        status: "noop".to_string(),
        reason: Some("empty_tree".to_string()),
        mode: None,
        context_mode: None,
        model: None,
        branches: Vec::new(),
        leaves_processed: 0,
        leaves_skipped: Vec::new(),
        notifications: vec!["pruned 3 orphan leaf records".to_string()],
    };

    let json = serde_json::to_string_pretty(&result).unwrap();
    assert!(json.contains("notifications"));
    assert!(json.contains("pruned 3 orphan leaf records"));

    let result_no_notifications = CompileResult {
        notifications: Vec::new(),
        ..result
    };
    let json_empty = serde_json::to_string_pretty(&result_no_notifications).unwrap();
    assert!(!json_empty.contains("notifications"));
}

fn branch_record(slug: &str, title: &str, leaf_slugs: &[&str]) -> BranchRecord {
    BranchRecord {
        slug: Slug::generate(slug, ""),
        file: format!("branches/{}.md", slug),
        title: title.to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        leaves: leaf_slugs.iter().map(|s| Slug::generate(s, "")).collect(),
    }
}

#[test]
fn repair_notifications_include_branch_repair_and_removal_messages() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));

    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-c",
        "leaf-c.md",
        "Leaf C",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-d",
        "leaf-d.md",
        "Leaf D",
        "2026-02-01T00:00:00Z",
    ));

    manifest
        .branches
        .push(branch_record("branch-1", "Branch 1", &["leaf-a", "leaf-b"]));
    manifest.branches.push(branch_record(
        "branch-2",
        "Branch 2",
        &["leaf-a", "leaf-b", "leaf-c"],
    ));
    manifest
        .branches
        .push(branch_record("branch-3", "Branch 3", &["leaf-c", "leaf-d"]));

    // Write only leaf-a and leaf-b; leaf-c and leaf-d are missing.
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
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications =
        plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    // Messages should include branch repair and removal, not just prune.
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

    // branch-1 (all leaves present): no repair needed, stays in manifest
    // branch-2 (leaf-c missing): repaired, stays at 2 leaves
    // branch-3 (both leaves missing): removed
    let repaired = read_manifest(dir.path());
    assert_eq!(repaired.branches.len(), 2);
    let branch_slugs: HashSet<&str> = repaired.branches.iter().map(|b| b.slug.as_str()).collect();
    assert!(branch_slugs.contains("branch-1"));
    assert!(branch_slugs.contains("branch-2"));
    assert!(!branch_slugs.contains("branch-3"));
}

#[test]
fn human_output_includes_notifications() {
    let result = CompileResult {
        status: "noop".to_string(),
        reason: Some("empty_tree".to_string()),
        mode: None,
        context_mode: None,
        model: None,
        branches: Vec::new(),
        leaves_processed: 0,
        leaves_skipped: Vec::new(),
        notifications: vec![
            "pruned 1 orphan leaf record (file missing, not in any branch)".to_string(),
        ],
    };
    let mut stdout = Vec::new();
    render_human(&result, &mut stdout, "test-tree").unwrap();
    let output = String::from_utf8(stdout).unwrap();

    assert!(output.contains("test-tree is empty"));
    assert!(output.contains("\u{2192} pruned 1 orphan"));
}

#[test]
fn derived_compile_schema_requires_branches() {
    let schema = super::schema::compile_response_schema();
    let obj = schema.as_object().expect("top-level is object");
    assert_eq!(obj["additionalProperties"], false);
    let required: Vec<&str> = obj["required"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(required.contains(&"branches"));
}

#[test]
fn derived_incremental_compile_schema_requires_updated_and_new_branches() {
    let schema = super::schema::incremental_compile_response_schema();
    let obj = schema.as_object().expect("top-level is object");
    assert_eq!(obj["additionalProperties"], false);
    let required: Vec<&str> = obj["required"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(required.contains(&"updated_branches"));
    assert!(required.contains(&"new_branches"));
}
