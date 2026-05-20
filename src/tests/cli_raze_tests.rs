use super::*;
use crate::domain::manifest::{self, LeafRecord, Manifest, TreeMeta};
use crate::domain::{Slug, Timestamp, Title, Url};
use crate::engine::config;
use std::fs;
use tempfile::TempDir;

fn setup_tree(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let tree_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");
    fs::create_dir_all(&tree_dir).unwrap();
    let bo_dir = tree_dir.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    manifest::write(
        &bo_dir.join("manifest.json"),
        &Manifest {
            tree: TreeMeta {
                name: "tree".to_string(),
                created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        },
    )
    .unwrap();

    config::write_config(
        &config::Config {
            tree: Some(crate::domain::tree::TreeConfig {
                output_dir: tree_dir.clone(),
                name: Some("tree".to_string()),
                created_at: Some("2025-01-01T00:00:00Z".to_string()),
            }),
            model: None,
            compile_model: None,
        },
        &config_path,
    )
    .unwrap();

    (tree_dir, config_path)
}

fn auth_path_for_config(config_path: &std::path::Path) -> std::path::PathBuf {
    config_path.with_file_name("auth.json")
}

fn add_leaf(tree_dir: &std::path::Path, file: &str) {
    add_manifest_leaf(tree_dir, file);
    fs::write(tree_dir.join(file), "# content\n").unwrap();
}

fn add_manifest_leaf(tree_dir: &std::path::Path, file: &str) {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let manifest_path = tree_dir.join(".bo/manifest.json");
    let mut manifest = manifest::read(&manifest_path).unwrap();
    // Use a valid slug even for adversarial file paths
    let idx = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let slug = Slug::parse(&format!("leaf-{}", idx)).unwrap();
    manifest.leaves.push(LeafRecord {
        slug,
        file: file.to_string(),
        title: Title::new(file.trim_end_matches(".md")),
        url: Url::parse("https://example.com/test").unwrap(),
        collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
        summary: None,
    });
    manifest::write(&manifest_path, &manifest).unwrap();
}

#[test]
fn deletes_indexed_files() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    add_leaf(&tree_dir, "a.md");
    add_leaf(&tree_dir, "b.md");

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.result.deleted_files, 2);
    assert!(!tree_dir.join("a.md").exists());
    assert!(!tree_dir.join("b.md").exists());
}

#[test]
fn deletes_index_file() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(output.result.deleted_index);
    assert!(!tree_dir.join(".bo").exists());
}

#[test]
fn deletes_manifest_alongside_other_infra() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    // Pre-T6.1, raze removes the entire .bo/ directory which incidentally
    // wipes the manifest. This test pins the behaviour and guards against
    // anyone reintroducing a manifest path that escapes infra teardown.
    let manifest_path = tree_dir.join(".bo/manifest.json");
    crate::domain::manifest::write(
        &manifest_path,
        &crate::domain::manifest::Manifest {
            tree: crate::domain::manifest::TreeMeta {
                name: "raze-test".to_string(),
                created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        },
    )
    .unwrap();
    assert!(manifest_path.exists());

    let _ = raze(&tree_dir, &config_path).unwrap();

    assert!(
        !manifest_path.exists(),
        "manifest.json must be deleted by raze"
    );
}

#[test]
fn removes_empty_output_directory() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(output.result.removed_output_dir);
    assert!(!tree_dir.exists());
}

#[test]
fn leaves_non_empty_directory_in_place() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    fs::write(tree_dir.join("stray.txt"), "not tracked").unwrap();

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(!output.result.removed_output_dir);
    assert!(output.result.output_dir_left_in_place);
    assert!(tree_dir.exists());
}

#[test]
fn deletes_config_file() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    assert!(config_path.exists());

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(output.result.deleted_config);
    assert!(!config_path.exists());
}

#[test]
fn preserves_auth_file_by_default() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    let auth_path = auth_path_for_config(&config_path);
    fs::write(
        &auth_path,
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(!output.result.deleted_auth);
    assert!(output.result.preserved_auth);
    assert_eq!(output.result.auth_path, auth_path.display().to_string());
    assert!(auth_path.exists());
}

#[test]
fn include_auth_deletes_auth_file() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    let auth_path = auth_path_for_config(&config_path);
    fs::write(
        &auth_path,
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();

    let output = raze_with_auth(&tree_dir, &config_path, &auth_path, AuthCleanup::Delete).unwrap();

    assert!(output.result.deleted_auth);
    assert!(!output.result.preserved_auth);
    assert_eq!(output.result.auth_path, auth_path.display().to_string());
    assert!(!auth_path.exists());
}

#[test]
fn missing_auth_file_is_tolerated() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(!output.result.deleted_auth);
    assert!(!output.result.preserved_auth);
}

#[test]
fn auth_only_cleanup_deletes_auth_without_tree_config() {
    let tmp = TempDir::new().unwrap();
    let auth_path = tmp.path().join("auth.json");
    fs::write(
        &auth_path,
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();

    let output = raze_auth_only(&auth_path).unwrap().unwrap();

    assert!(output.result.deleted_auth);
    assert!(!output.result.deleted_config);
    assert!(!auth_path.exists());
    assert!(render_human(&output.result).contains("deleted auth"));
}

#[test]
fn skips_missing_files_without_error() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    add_manifest_leaf(&tree_dir, "ghost.md");

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.result.deleted_files, 0);
    assert!(output.warnings.is_empty());
}

#[test]
fn warns_on_suspicious_path_traversal() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    add_manifest_leaf(&tree_dir, "../escape.md");

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.warnings.len(), 1);
    assert_eq!(output.warnings[0].code, "suspicious_manifest_entry");
    assert_eq!(output.result.deleted_files, 0);
}

#[test]
fn warns_on_absolute_path_in_manifest() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    add_manifest_leaf(&tree_dir, "/etc/passwd");

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.warnings.len(), 1);
    assert_eq!(output.warnings[0].code, "suspicious_manifest_entry");
}

#[test]
fn empty_tree_produces_zero_deletes() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.result.deleted_files, 0);
    assert!(output.result.deleted_index);
}

#[test]
fn render_human_includes_file_count() {
    let result = RazeResult {
        deleted_files: 3,
        deleted_index: true,
        removed_output_dir: true,
        output_dir_left_in_place: false,
        deleted_config: true,
        deleted_auth: true,
        preserved_auth: false,
        output_dir: "/tmp/tree".to_string(),
        config_path: "/tmp/.bo/config.json".to_string(),
        auth_path: "/tmp/.bo/auth.json".to_string(),
    };
    let output = render_human(&result);
    assert!(output.contains("3 markdown file(s)"));
    assert!(output.contains("deleted index"));
    assert!(output.contains("removed output directory"));
    assert!(output.contains("deleted config"));
    assert!(output.contains("deleted auth"));
}

#[test]
fn render_human_shows_dir_left_in_place() {
    let result = RazeResult {
        deleted_files: 0,
        deleted_index: false,
        removed_output_dir: false,
        output_dir_left_in_place: true,
        deleted_config: false,
        deleted_auth: false,
        preserved_auth: false,
        output_dir: "/tmp/tree".to_string(),
        config_path: "/tmp/.bo/config.json".to_string(),
        auth_path: "/tmp/.bo/auth.json".to_string(),
    };
    let output = render_human(&result);
    assert!(output.contains("left in place"));
}

#[test]
fn render_human_shows_preserved_auth() {
    let result = RazeResult {
        deleted_files: 0,
        deleted_index: false,
        removed_output_dir: false,
        output_dir_left_in_place: false,
        deleted_config: true,
        deleted_auth: false,
        preserved_auth: true,
        output_dir: "/tmp/tree".to_string(),
        config_path: "/tmp/.bo/config.json".to_string(),
        auth_path: "/tmp/.bo/auth.json".to_string(),
    };
    let output = render_human(&result);
    assert!(output.contains("preserved auth"));
    assert!(output.contains("/tmp/.bo/auth.json"));
}
