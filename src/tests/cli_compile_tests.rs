use super::{plan, render_human, CompileResult};
use crate::domain::manifest::{LeafRecord, Manifest, TreeMeta};
use crate::domain::{Slug, Timestamp, Title, Url};
use crate::engine::config::SeededConfig;
use std::path::Path;
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

fn seeded_config(dir: &Path) -> SeededConfig {
    let tree_cfg = crate::domain::tree::TreeConfig {
        output_dir: dir.to_path_buf(),
        name: Some("test-tree".to_string()),
        created_at: Some("2026-01-01T00:00:00Z".to_string()),
    };
    let config = crate::engine::config::Config {
        tree: Some(tree_cfg.clone()),
        ..crate::engine::config::Config::default()
    };
    let mut seeded = config.into_seeded().expect("seeded config");
    seeded.tree_cfg = tree_cfg;
    seeded
}

fn write_manifest(dir: &Path, manifest: &Manifest) {
    let tree = crate::domain::tree::Tree::from_config(&crate::domain::tree::TreeConfig {
        output_dir: dir.to_path_buf(),
        name: Some("test-tree".to_string()),
        created_at: Some("2026-01-01T00:00:00Z".to_string()),
    });
    crate::domain::manifest::write(&tree.manifest_path(), manifest).unwrap();
}

fn read_manifest(dir: &Path) -> Manifest {
    let manifest_path = crate::domain::tree::manifest_path(dir);
    crate::domain::manifest::read(&manifest_path).unwrap()
}

fn leaf_record(slug: &str, file: &str, title: &str, collected_at: &str) -> LeafRecord {
    LeafRecord {
        slug: Slug::generate(slug, ""),
        file: file.to_string(),
        title: Title::new(title),
        url: Url::parse("https://example.com").unwrap(),
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
    let result = plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert!(result.manifest_changed);
    assert_eq!(result.deleted_leaf_slugs.len(), 1);
    assert!(result.orphan_leaf_slugs.contains(&"leaf-a".to_string()));
    assert_eq!(result.notifications.len(), 1);
    assert!(result.notifications[0].contains("pruned 1 orphan"));

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
    let result = plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert!(result.manifest_changed);
    assert_eq!(result.notifications.len(), 1);
    assert!(result.notifications[0].contains("pruned 1 orphan"));
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
    let result = plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert!(!result.manifest_changed);
    assert!(result.notifications.is_empty());
    assert!(result.orphan_leaf_slugs.is_empty());
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
    let result = plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert!(result.manifest_changed);
    assert_eq!(result.notifications.len(), 1);
    assert!(result.notifications[0].contains("pruned 2 orphan"));
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
