// Commit-stage tests: dedup/slug/state/pending helpers exercised directly,
// without driving the full pipeline. Stage-unit tests for input/compute/journal
// live in their own files.

use super::*;
use crate::domain::state::{TreeMetadata, TreeState};
use crate::domain::{Slug, Timestamp, Title, Url};
use std::fs;
use tempfile::TempDir;

#[test]
fn dedup_reads_state_not_legacy_index() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let state_path = dir.path().join(".bo/state.json");

    // Seed the state with one leaf (mirrors seed_for_collect + a pushed
    // leaf) but write no legacy index file. The dedup path (duplicate_file)
    // must consult the state, not the legacy index.
    let m = TreeState {
        tree: TreeMetadata {
            name: "state-dedup-tree".to_string(),
            created_at: Timestamp::parse("2026-05-19T12:00:00Z").unwrap(),
            last_compiled_at: None,
        },
        leaves: vec![crate::domain::Leaf {
            slug: Slug::parse("already-collected").unwrap(),
            file: "already-collected.md".to_string(),
            title: Some(Title::parse("Already").unwrap()),
            url: Url::parse("https://example.com/article").unwrap(),
            collected_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            summary: None,
        }],
        branches: Vec::new(),
    };
    crate::engine::state::write(&state_path, &m).unwrap();

    // Verify duplicate_file (the dedup helper) finds it via state only.
    let existing = duplicate_file("https://example.com/article", dir.path()).unwrap();
    assert_eq!(existing.as_deref(), Some("already-collected.md"));

    // Sanity: a different URL is not flagged.
    let none = duplicate_file("https://example.com/other", dir.path()).unwrap();
    assert!(none.is_none());
}

#[test]
fn commit_rejects_legacy_state_before_writing_pending_or_state() {
    let dir = TempDir::new().unwrap();
    let bo_dir = dir.path().join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    fs::write(bo_dir.join("manifest.json"), b"legacy").unwrap();
    let state = TreeState {
        tree: TreeMetadata {
            name: "legacy-tree".to_string(),
            created_at: Timestamp::parse("2026-05-19T12:00:00Z").unwrap(),
            last_compiled_at: None,
        },
        leaves: Vec::new(),
        branches: Vec::new(),
    };

    let error = commit_state_and_writes(
        dir.path(),
        OpKind::Collect {
            url: "https://example.com/article".to_string(),
        },
        &state,
        &[],
        &[],
    )
    .unwrap_err();

    assert!(matches!(
        error,
        CollectError::TreeState(crate::domain::state::TreeStateError::LegacyManifestFound)
    ));
    assert!(!bo_dir.join("pending.json").exists());
    assert!(!bo_dir.join("state.json").exists());
}
