use super::*;
use crate::domain::manifest::{Manifest, ManifestError, TreeMeta};
use crate::domain::tree::{Tree, TreeConfig};
use crate::domain::{Branch, Leaf, Title, Url};
use crate::domain::{Slug, Timestamp};
use std::path::PathBuf;
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

fn sample_manifest() -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: "rust-notes".to_string(),
            created_at: Timestamp::parse("2026-05-19T14:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2026-05-19T14:32:11.000Z").unwrap()),
        },
        leaves: vec![Leaf {
            slug: Slug::parse("ownership-and-borrowing").unwrap(),
            file: "ownership-and-borrowing.md".to_string(),
            title: Some(Title::parse("Ownership and Borrowing").unwrap()),
            url: Url::parse("https://example.com/ownership").unwrap(),
            collected_at: Timestamp::parse("2026-05-19T14:05:32Z").unwrap(),
            summary: Some("Rust's ownership rules.".to_string()),
        }],
        branches: vec![Branch {
            slug: Slug::parse("memory-safety").unwrap(),
            file: "branches/memory-safety.md".to_string(),
            title: Title::parse("Memory Safety").unwrap(),
            created_at: Timestamp::parse("2026-05-19T14:32:11.000Z").unwrap(),
            updated_at: Timestamp::parse("2026-05-19T14:32:11.000Z").unwrap(),
            leaves: vec![Slug::parse("ownership-and-borrowing").unwrap()],
        }],
    }
}

fn empty_manifest() -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: "empty-tree".to_string(),
            created_at: Timestamp::parse("2026-05-19T14:00:00Z").unwrap(),
            last_compiled_at: None,
        },
        leaves: Vec::new(),
        branches: Vec::new(),
    }
}

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

// ── round-trip ───────────────────────────────────────────────────────────────

#[test]
fn round_trip_empty_manifest() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".bo/manifest.json");
    let original = empty_manifest();

    write(&path, &original).unwrap();
    let loaded = read(&path).unwrap();

    assert_eq!(loaded, original);
}

#[test]
fn round_trip_full_manifest_preserves_all_fields() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".bo/manifest.json");
    let original = sample_manifest();

    write(&path, &original).unwrap();
    let loaded = read(&path).unwrap();

    assert_eq!(loaded, original);
    assert_eq!(
        loaded.branches[0].created_at.to_string(),
        "2026-05-19T14:32:11.000Z"
    );
    assert_eq!(
        loaded.branches[0].updated_at.to_string(),
        "2026-05-19T14:32:11.000Z"
    );
    assert_eq!(
        loaded.leaves[0].summary.as_deref(),
        Some("Rust's ownership rules.")
    );
}

#[test]
fn write_produces_pretty_printed_json() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".bo/manifest.json");
    write(&path, &sample_manifest()).unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    // Pretty-printed JSON has line breaks and indentation.
    assert!(content.contains('\n'));
    assert!(content.contains("  \"tree\""));
}

#[test]
fn write_creates_parent_directory_if_missing() {
    let dir = TempDir::new().unwrap();
    // .bo does not exist yet
    let path = dir.path().join(".bo/manifest.json");
    assert!(!dir.path().join(".bo").exists());

    write(&path, &empty_manifest()).unwrap();

    assert!(path.exists());
}

#[test]
fn write_overwrites_existing_manifest() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".bo/manifest.json");

    write(&path, &empty_manifest()).unwrap();
    write(&path, &sample_manifest()).unwrap();

    let loaded = read(&path).unwrap();
    assert_eq!(loaded, sample_manifest());
}

#[test]
fn write_does_not_leak_tmp_file_on_success() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".bo/manifest.json");
    write(&path, &sample_manifest()).unwrap();

    let tmp_path = format!("{}.tmp", path.display());
    assert!(
        !PathBuf::from(&tmp_path).exists(),
        "tmp file leaked: {tmp_path}"
    );
    assert!(path.exists());
}

// ── debug-only invariant checks ──────────────────────────────────────────────

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "duplicate leaf slug")]
fn write_panics_on_duplicate_leaf_slugs_in_debug() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".bo/manifest.json");
    let mut m = sample_manifest();
    let dup = m.leaves[0].clone();
    m.leaves.push(dup);

    let _ = write(&path, &m);
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "duplicate branch slug")]
fn write_panics_on_duplicate_branch_slugs_in_debug() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".bo/manifest.json");
    let mut m = sample_manifest();
    let dup = m.branches[0].clone();
    m.branches.push(dup);

    let _ = write(&path, &m);
}

// ── read errors ──────────────────────────────────────────────────────────────

#[test]
fn read_returns_parse_error_on_invalid_json() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".bo/manifest.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{not valid json").unwrap();

    let err = read(&path).unwrap_err();
    assert!(matches!(err, ManifestError::Parse(_)), "got: {err}");
}

#[test]
fn read_returns_tree_not_initialized_when_file_absent() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join(".bo/manifest.json");
    assert!(!path.exists());

    let err = read(&path).unwrap_err();
    assert!(
        matches!(err, ManifestError::TreeNotInitialized),
        "got: {err}"
    );
}

// ── runtime state ────────────────────────────────────────────────────────────

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
    write(
        &crate::domain::tree::manifest_path(dir.path()),
        &tree.empty_manifest(),
    )
    .unwrap();
    assert!(matches!(
        runtime_state(dir.path()).unwrap(),
        TreeRuntimeState::Initialized(_)
    ));
}

#[test]
fn fresh_manifest_uses_tree_metadata() {
    let dir = TempDir::new().unwrap();
    let tree = Tree::from_config(&temp_config(&dir));

    let manifest = crate::engine::manifest::manifest_or_empty_if_fresh(&tree).unwrap();

    assert_eq!(manifest.tree.name, "my-research");
    assert_eq!(manifest.tree.created_at, timestamp());
    assert!(manifest.leaves.is_empty());
    assert!(manifest.branches.is_empty());
}
