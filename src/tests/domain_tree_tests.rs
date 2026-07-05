use super::*;
use crate::domain::Timestamp;
use std::path::PathBuf;

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

    assert_eq!(
        crate::domain::tree::infra_dir(&tree.path),
        PathBuf::from("/tmp/my-research/.bo")
    );
}

#[test]
fn manifest_path_is_path_slash_bo_manifest_json() {
    let tree = Tree::from_config(&full_config());

    assert_eq!(
        crate::domain::tree::manifest_path(&tree.path),
        PathBuf::from("/tmp/my-research/.bo/manifest.json")
    );
}

#[test]
fn free_manifest_path_matches_tree_method() {
    let tree = Tree::from_config(&full_config());

    assert_eq!(
        crate::domain::tree::manifest_path(&tree.path),
        super::manifest_path(&tree.path)
    );
}
