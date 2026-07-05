use super::*;
use crate::domain::Timestamp;
use std::fs;
use std::io::Cursor;
use tempfile::TempDir;

#[path = "fixtures/mod.rs"]
mod fixtures;

use fixtures::{
    add_leaf, add_manifest_leaf, auth_path_for_config, make_leaf_record, make_manifest, setup_tree,
};

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

    assert!(output.result.deleted_manifest);
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
    crate::engine::manifest::write(
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
    assert!(output.result.deleted_manifest);
}

#[test]
fn render_human_includes_file_count() {
    let result = RazeResult {
        cancelled: false,
        deleted_files: 3,
        deleted_manifest: true,
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
    assert!(output.contains("deleted manifest"));
    assert!(output.contains("removed output directory"));
    assert!(output.contains("deleted config"));
    assert!(output.contains("deleted auth"));
}

#[test]
fn render_human_shows_dir_left_in_place() {
    let result = RazeResult {
        cancelled: false,
        deleted_files: 0,
        deleted_manifest: false,
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
        cancelled: false,
        deleted_files: 0,
        deleted_manifest: false,
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

// ─── confirmation gate tests ────────────────────────────────────────────────

#[test]
fn confirm_accepts_exact_yes() {
    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    let result = confirm_raze(
        std::path::Path::new("/tmp/tree"),
        None,
        false,
        &mut reader,
        &mut writer,
    )
    .unwrap();
    assert!(result);
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("Type 'yes' to confirm:"));
}

#[test]
fn confirm_rejects_variants() {
    for input in &["Yes\n", "YES\n", "y\n", "\n"] {
        let mut reader = Cursor::new(input.as_bytes());
        let mut writer = Vec::new();
        let result = confirm_raze(
            std::path::Path::new("/tmp/tree"),
            None,
            false,
            &mut reader,
            &mut writer,
        )
        .unwrap();
        assert!(!result, "should reject {:?}", input);
    }
}

#[test]
fn confirm_rejects_random() {
    for input in &["no\n", "maybe\n", "foo\n", "yes \n"] {
        let mut reader = Cursor::new(input.as_bytes());
        let mut writer = Vec::new();
        let result = confirm_raze(
            std::path::Path::new("/tmp/tree"),
            None,
            false,
            &mut reader,
            &mut writer,
        )
        .unwrap();
        assert!(!result, "should reject {:?}", input);
    }
}

#[test]
fn confirm_shows_tree_path() {
    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    confirm_raze(
        std::path::Path::new("/home/user/my-tree"),
        None,
        false,
        &mut reader,
        &mut writer,
    )
    .unwrap();
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("/home/user/my-tree"));
}

#[test]
fn confirm_shows_leaf_and_branch_counts() {
    let manifest = make_manifest(
        "test",
        vec![
            make_leaf_record("leaf-1", "a.md"),
            make_leaf_record("leaf-2", "b.md"),
        ],
    );

    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    confirm_raze(
        std::path::Path::new("/tmp/tree"),
        Some(&manifest),
        false,
        &mut reader,
        &mut writer,
    )
    .unwrap();
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("2 leaves, 0 branches"));
}

#[test]
fn confirm_missing_manifest_shows_degraded_message() {
    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    confirm_raze(
        std::path::Path::new("/tmp/tree"),
        None,
        false,
        &mut reader,
        &mut writer,
    )
    .unwrap();
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("unable to read manifest"));
}

#[test]
fn confirm_with_auth_shows_will_be_deleted() {
    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    confirm_raze(
        std::path::Path::new("/tmp/tree"),
        None,
        true,
        &mut reader,
        &mut writer,
    )
    .unwrap();
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("will be deleted"));
}

#[test]
fn render_human_cancelled_shows_message() {
    let result = RazeResult {
        cancelled: true,
        deleted_files: 0,
        deleted_manifest: false,
        removed_output_dir: false,
        output_dir_left_in_place: false,
        deleted_config: false,
        deleted_auth: false,
        preserved_auth: false,
        output_dir: String::new(),
        config_path: String::new(),
        auth_path: String::new(),
    };
    let output = render_human(&result);
    assert_eq!(output, "raze cancelled\n");
}
