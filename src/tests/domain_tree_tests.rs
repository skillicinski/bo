use super::*;
use std::path::PathBuf;

fn full_config() -> TreeConfig {
    TreeConfig {
        output_dir: PathBuf::from("/tmp/my-research"),
        name: Some("my-research".to_string()),
        created_at: Some("2026-04-14T09:00:00Z".to_string()),
    }
}

fn old_config() -> TreeConfig {
    TreeConfig {
        output_dir: PathBuf::from("/tmp/old-tree"),
        name: None,
        created_at: None,
    }
}

#[test]
fn from_full_config_preserves_all_fields() {
    let tree = Tree::from_config(&full_config());
    assert_eq!(tree.name.as_deref(), Some("my-research"));
    assert_eq!(tree.created_at.as_deref(), Some("2026-04-14T09:00:00Z"));
    assert_eq!(tree.output_dir, PathBuf::from("/tmp/my-research"));
}

#[test]
fn from_old_config_derives_name_from_dir_basename() {
    let tree = Tree::from_config(&old_config());
    // name is derived from "old-tree" (basename of /tmp/old-tree)
    assert_eq!(tree.name.as_deref(), Some("old-tree"));
    assert!(tree.created_at.is_none());
}

#[test]
fn from_config_name_explicit_beats_derived() {
    // When config.name is set, it wins over the directory basename
    let config = TreeConfig {
        output_dir: PathBuf::from("/tmp/dir-name"),
        name: Some("explicit-name".to_string()),
        created_at: None,
    };
    let tree = Tree::from_config(&config);
    assert_eq!(tree.name.as_deref(), Some("explicit-name"));
}

#[test]
fn branches_dir_is_output_dir_slash_branches() {
    let tree = Tree::from_config(&full_config());
    assert_eq!(
        tree.branches_dir(),
        PathBuf::from("/tmp/my-research/branches")
    );
}
