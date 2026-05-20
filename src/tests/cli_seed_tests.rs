use super::*;
use crate::domain::manifest;
use crate::domain::tree::Tree;
use crate::engine::config;
use tempfile::TempDir;

#[test]
fn creates_output_directory_and_config() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");

    let result = seed(output_dir.clone(), None, &config_path).unwrap();

    assert_eq!(result.status, "created");
    assert!(output_dir.exists());
    let cfg = config::read_config(&config_path).unwrap();
    let tree = cfg.tree.unwrap();
    assert_eq!(tree.output_dir, output_dir);
}

#[test]
fn derives_name_from_directory_basename() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("my-tree");
    let config_path = tmp.path().join("config.json");

    let result = seed(output_dir, None, &config_path).unwrap();

    assert_eq!(result.tree_name.as_deref(), Some("my-tree"));
    let cfg = config::read_config(&config_path).unwrap();
    let tree = cfg.tree.unwrap();
    assert_eq!(tree.name.as_deref(), Some("my-tree"));
}

#[test]
fn explicit_name_overrides_basename() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("some-dir");
    let config_path = tmp.path().join("config.json");

    let result = seed(output_dir, Some("custom".to_string()), &config_path).unwrap();

    assert_eq!(result.tree_name.as_deref(), Some("custom"));
    let cfg = config::read_config(&config_path).unwrap();
    let tree = cfg.tree.unwrap();
    assert_eq!(tree.name.as_deref(), Some("custom"));
}

#[test]
fn sets_created_at_timestamp() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");

    seed(output_dir, None, &config_path).unwrap();

    let cfg = config::read_config(&config_path).unwrap();
    let tree = cfg.tree.unwrap();
    assert!(tree.created_at.is_some());
}

#[test]
fn already_seeded_returns_existing_config() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");

    let first = seed(output_dir.clone(), None, &config_path).unwrap();
    assert_eq!(first.status, "created");

    let second = seed(output_dir, None, &config_path).unwrap();
    assert_eq!(second.status, "already_seeded");
}

#[test]
fn seeds_config_without_tree_and_preserves_model() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");

    config::write_config(
        &config::Config {
            tree: None,
            model: Some("gpt-4.1-mini".to_string()),
            compile_model: None,
        },
        &config_path,
    )
    .unwrap();

    let result = seed(output_dir.clone(), None, &config_path).unwrap();

    assert_eq!(result.status, "created");
    let cfg = config::read_config(&config_path).unwrap();
    assert_eq!(cfg.model.as_deref(), Some("gpt-4.1-mini"));
    assert_eq!(cfg.tree.unwrap().output_dir, output_dir);
}

#[test]
fn idempotent_does_not_update_created_at() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");

    seed(output_dir.clone(), None, &config_path).unwrap();
    let first_ts = config::read_config(&config_path)
        .unwrap()
        .tree
        .unwrap()
        .created_at;

    seed(output_dir, None, &config_path).unwrap();
    let second_ts = config::read_config(&config_path)
        .unwrap()
        .tree
        .unwrap()
        .created_at;

    assert_eq!(first_ts, second_ts);
}

#[test]
fn resolves_relative_path_to_absolute() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("relative-tree");
    let config_path = tmp.path().join("config.json");

    let result = seed(output_dir, None, &config_path).unwrap();

    assert!(result.output_dir.starts_with('/'));
    assert!(result.output_dir.contains("relative-tree"));
}

#[test]
fn render_human_created() {
    let result = SeedResult {
        status: "created".to_string(),
        output_dir: "/tmp/tree".to_string(),
        tree_name: Some("tree".to_string()),
    };
    assert_eq!(render_human(&result), "seeded bo at /tmp/tree");
}

#[test]
fn render_human_already_seeded() {
    let result = SeedResult {
        status: "already_seeded".to_string(),
        output_dir: "/tmp/tree".to_string(),
        tree_name: Some("tree".to_string()),
    };
    assert_eq!(
        render_human(&result),
        "bo has already been seeded at /tmp/tree!"
    );
}

// ── manifest dual-write (T4.1) ────────────────────────────────────────────────────

#[test]
fn writes_empty_manifest_to_infra_dir() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");

    seed(output_dir.clone(), None, &config_path).unwrap();

    let manifest_path = output_dir.join(".bo/manifest.json");
    assert!(
        manifest_path.exists(),
        "manifest.json should exist after seed"
    );
}

#[test]
fn manifest_metadata_matches_config() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");

    seed(
        output_dir.clone(),
        Some("my-tree".to_string()),
        &config_path,
    )
    .unwrap();

    let cfg = config::read_config(&config_path).unwrap();
    let tree_cfg = cfg.tree.unwrap();
    let tree = Tree::from_config(&tree_cfg);
    let m = manifest::read(&tree.manifest_path()).unwrap();

    assert_eq!(m.tree.name, "my-tree");
    assert_eq!(m.tree.created_at, tree_cfg.created_at.unwrap());
    assert!(m.tree.last_compiled_at.is_none());
    assert!(m.leaves.is_empty());
    assert!(m.branches.is_empty());
}

#[test]
fn manifest_not_overwritten_on_already_seeded() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");

    seed(output_dir.clone(), None, &config_path).unwrap();
    let manifest_path = output_dir.join(".bo/manifest.json");
    let first = std::fs::read_to_string(&manifest_path).unwrap();

    // Re-running seed should be a no-op for the manifest.
    seed(output_dir.clone(), None, &config_path).unwrap();
    let second = std::fs::read_to_string(&manifest_path).unwrap();

    assert_eq!(first, second);
}
