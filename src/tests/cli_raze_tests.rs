use super::*;
use crate::domain::index::{self, IndexEntry};
use crate::engine::config;
use std::fs;
use tempfile::TempDir;

fn setup_tree(tmp: &TempDir) -> (std::path::PathBuf, std::path::PathBuf) {
    let tree_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");
    fs::create_dir_all(&tree_dir).unwrap();
    fs::write(tree_dir.join("index.jsonl"), "").unwrap();

    config::write_config(
        &config::Config {
            tree: crate::domain::tree::TreeConfig {
                output_dir: tree_dir.clone(),
                name: Some("tree".to_string()),
                created_at: Some("2025-01-01T00:00:00Z".to_string()),
            },
            compile_model: None,
        },
        &config_path,
    )
    .unwrap();

    (tree_dir, config_path)
}

fn add_leaf(tree_dir: &std::path::Path, file: &str) {
    index::append_entry(
        &tree_dir.join("index.jsonl"),
        &IndexEntry {
            file: file.to_string(),
            title: file.trim_end_matches(".md").to_string(),
            url: format!("https://example.com/{}", file),
        },
    )
    .unwrap();
    fs::write(tree_dir.join(file), "# content\n").unwrap();
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
    assert!(!tree_dir.join("index.jsonl").exists());
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
fn skips_missing_files_without_error() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    index::append_entry(
        &tree_dir.join("index.jsonl"),
        &IndexEntry {
            file: "ghost.md".to_string(),
            title: "Ghost".to_string(),
            url: "https://example.com/ghost".to_string(),
        },
    )
    .unwrap();

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.result.deleted_files, 0);
    assert!(output.warnings.is_empty());
}

#[test]
fn warns_on_suspicious_path_traversal() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    index::append_entry(
        &tree_dir.join("index.jsonl"),
        &IndexEntry {
            file: "../escape.md".to_string(),
            title: "Escape".to_string(),
            url: "https://example.com/escape".to_string(),
        },
    )
    .unwrap();

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.warnings.len(), 1);
    assert_eq!(output.warnings[0].code, "suspicious_ledger_entry");
    assert_eq!(output.result.deleted_files, 0);
}

#[test]
fn warns_on_absolute_path_in_index() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    index::append_entry(
        &tree_dir.join("index.jsonl"),
        &IndexEntry {
            file: "/etc/passwd".to_string(),
            title: "Bad".to_string(),
            url: "https://example.com/bad".to_string(),
        },
    )
    .unwrap();

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.warnings.len(), 1);
    assert_eq!(output.warnings[0].code, "suspicious_ledger_entry");
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
        output_dir: "/tmp/tree".to_string(),
        config_path: "/tmp/.bo/config.json".to_string(),
    };
    let output = render_human(&result);
    assert!(output.contains("3 markdown file(s)"));
    assert!(output.contains("deleted index"));
    assert!(output.contains("removed output directory"));
    assert!(output.contains("deleted config"));
}

#[test]
fn render_human_shows_dir_left_in_place() {
    let result = RazeResult {
        deleted_files: 0,
        deleted_index: false,
        removed_output_dir: false,
        output_dir_left_in_place: true,
        deleted_config: false,
        output_dir: "/tmp/tree".to_string(),
        config_path: "/tmp/.bo/config.json".to_string(),
    };
    let output = render_human(&result);
    assert!(output.contains("left in place"));
}
