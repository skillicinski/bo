use super::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn sample_manifest() -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: "rust-notes".to_string(),
            created_at: "2026-05-19T14:00:00Z".to_string(),
            last_compiled_at: Some("2026-05-19T14:32:11Z".to_string()),
        },
        leaves: vec![LeafRecord {
            slug: "ownership-and-borrowing".to_string(),
            file: "ownership-and-borrowing.md".to_string(),
            title: "Ownership and Borrowing".to_string(),
            url: "https://example.com/ownership".to_string(),
            collected_at: "2026-05-19T14:05:32Z".to_string(),
            summary: Some("Rust's ownership rules.".to_string()),
        }],
        branches: vec![BranchRecord {
            slug: "memory-safety".to_string(),
            file: "branches/memory-safety.md".to_string(),
            title: "Memory Safety".to_string(),
            created_at: "2026-05-19T14:32:11Z".to_string(),
            updated_at: "2026-05-19T14:32:11Z".to_string(),
            stale: false,
            leaves: vec!["ownership-and-borrowing".to_string()],
        }],
    }
}

fn empty_manifest() -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: "empty-tree".to_string(),
            created_at: "2026-05-19T14:00:00Z".to_string(),
            last_compiled_at: None,
        },
        leaves: Vec::new(),
        branches: Vec::new(),
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
    assert_eq!(loaded.branches[0].created_at, "2026-05-19T14:32:11Z");
    assert_eq!(loaded.branches[0].updated_at, "2026-05-19T14:32:11Z");
    assert!(!loaded.branches[0].stale);
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

    let content = fs::read_to_string(&path).unwrap();
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
        !Path::new(&tmp_path).exists(),
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
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, "{not valid json").unwrap();

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
