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
fn infra_dir_is_path_slash_bo() {
    let tree = Tree::from_config(&full_config());

    assert_eq!(
        crate::domain::tree::infra_dir(&tree.path),
        PathBuf::from("/tmp/my-research/.bo")
    );
}

#[test]
fn state_path_is_path_slash_bo_state_json() {
    let tree = Tree::from_config(&full_config());

    assert_eq!(
        crate::domain::tree::state_path(&tree.path),
        PathBuf::from("/tmp/my-research/.bo/state.json")
    );
}

// ── round-trip: on-disk JSON shape unchanged ──────────────────────────────

#[test]
fn tree_config_round_trip_preserves_json_shape() {
    let json = r#"{
  "path": "/tmp/my-research",
  "name": "my-research",
  "created_at": "2026-04-14T09:00:00.000Z"
}"#;

    let cfg: TreeConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.path, PathBuf::from("/tmp/my-research"));
    assert_eq!(cfg.name, "my-research");
    assert_eq!(cfg.created_at.to_string(), "2026-04-14T09:00:00.000Z");

    let round_tripped = serde_json::to_string_pretty(&cfg).unwrap();
    assert_eq!(round_tripped, json);
}

#[test]
fn tree_meta_round_trip_preserves_json_shape() {
    let json = r#"{
  "name": "my-research",
  "created_at": "2026-04-14T09:00:00.000Z",
  "last_compiled_at": "2026-04-14T10:00:00.000Z"
}"#;

    use crate::domain::state::TreeMetadata;
    let meta: TreeMetadata = serde_json::from_str(json).unwrap();
    assert_eq!(meta.name, "my-research");
    assert_eq!(meta.created_at.to_string(), "2026-04-14T09:00:00.000Z");
    assert_eq!(
        meta.last_compiled_at.as_ref().unwrap().to_string(),
        "2026-04-14T10:00:00.000Z"
    );

    let round_tripped = serde_json::to_string_pretty(&meta).unwrap();
    assert_eq!(round_tripped, json);
}
