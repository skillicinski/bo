use super::*;
use crate::domain::Timestamp;
use std::path::PathBuf;
use tempfile::TempDir;

fn timestamp() -> Timestamp {
    Timestamp::parse("2026-04-14T09:00:00Z").unwrap()
}

fn full_config() -> TreeConfig {
    TreeConfig {
        path: PathBuf::from("/tmp/my-research"),
        name: "my-research".to_string(),
        created_at: timestamp(),
    }
}

fn temp_config(dir: &TempDir) -> TreeConfig {
    TreeConfig {
        path: dir.path().to_path_buf(),
        ..full_config()
    }
}

#[test]
fn tree_uses_config_metadata_directly() {
    let tree = Tree::from_config(&full_config());

    assert_eq!(tree.name, "my-research");
    assert_eq!(tree.created_at, timestamp());
    assert_eq!(tree.path, PathBuf::from("/tmp/my-research"));
}

#[test]
fn branches_dir_is_path_slash_branches() {
    let tree = Tree::from_config(&full_config());

    assert_eq!(
        tree.branches_dir(),
        PathBuf::from("/tmp/my-research/branches")
    );
}

#[test]
fn infra_dir_is_path_slash_bo() {
    let tree = Tree::from_config(&full_config());

    assert_eq!(tree.infra_dir(), PathBuf::from("/tmp/my-research/.bo"));
}

#[test]
fn manifest_path_is_path_slash_bo_manifest_json() {
    let tree = Tree::from_config(&full_config());

    assert_eq!(
        tree.manifest_path(),
        PathBuf::from("/tmp/my-research/.bo/manifest.json")
    );
}

#[test]
fn free_manifest_path_matches_tree_method() {
    let tree = Tree::from_config(&full_config());

    assert_eq!(tree.manifest_path(), super::manifest_path(&tree.path));
}

#[test]
fn runtime_state_distinguishes_fresh_missing_and_initialized() {
    let dir = TempDir::new().unwrap();

    assert!(matches!(
        runtime_state(dir.path()).unwrap(),
        TreeRuntimeState::FreshSeeded
    ));

    std::fs::create_dir_all(dir.path().join(".bo")).unwrap();
    assert!(matches!(
        runtime_state(dir.path()).unwrap(),
        TreeRuntimeState::MissingManifest
    ));

    let tree = Tree::from_config(&temp_config(&dir));
    crate::domain::manifest::write(&tree.manifest_path(), &tree.empty_manifest()).unwrap();
    assert!(matches!(
        runtime_state(dir.path()).unwrap(),
        TreeRuntimeState::Initialized(_)
    ));
}

#[test]
fn fresh_manifest_uses_tree_metadata() {
    let dir = TempDir::new().unwrap();
    let tree = Tree::from_config(&temp_config(&dir));

    let manifest = tree.manifest_or_empty_if_fresh().unwrap();

    assert_eq!(manifest.tree.name, "my-research");
    assert_eq!(manifest.tree.created_at, timestamp());
    assert!(manifest.leaves.is_empty());
    assert!(manifest.branches.is_empty());
}
