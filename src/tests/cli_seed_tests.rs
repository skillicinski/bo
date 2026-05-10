use super::*;
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
    assert_eq!(cfg.tree.output_dir, output_dir);
}

#[test]
fn derives_name_from_directory_basename() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("my-tree");
    let config_path = tmp.path().join("config.json");

    let result = seed(output_dir, None, &config_path).unwrap();

    assert_eq!(result.tree_name.as_deref(), Some("my-tree"));
    let cfg = config::read_config(&config_path).unwrap();
    assert_eq!(cfg.tree.name.as_deref(), Some("my-tree"));
}

#[test]
fn explicit_name_overrides_basename() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("some-dir");
    let config_path = tmp.path().join("config.json");

    let result = seed(output_dir, Some("custom".to_string()), &config_path).unwrap();

    assert_eq!(result.tree_name.as_deref(), Some("custom"));
    let cfg = config::read_config(&config_path).unwrap();
    assert_eq!(cfg.tree.name.as_deref(), Some("custom"));
}

#[test]
fn sets_created_at_timestamp() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");

    seed(output_dir, None, &config_path).unwrap();

    let cfg = config::read_config(&config_path).unwrap();
    assert!(cfg.tree.created_at.is_some());
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
fn idempotent_does_not_update_created_at() {
    let tmp = TempDir::new().unwrap();
    let output_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");

    seed(output_dir.clone(), None, &config_path).unwrap();
    let first_ts = config::read_config(&config_path).unwrap().tree.created_at;

    seed(output_dir, None, &config_path).unwrap();
    let second_ts = config::read_config(&config_path).unwrap().tree.created_at;

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
