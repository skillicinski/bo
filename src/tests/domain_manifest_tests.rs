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

fn resolution_fixture() -> Manifest {
    // last compile at T=15:00
    // alpha + beta collected before → "compiled"
    // gamma collected after  → "uncompiled"
    // topic-x: [alpha, beta], topic-y: [beta] (shared with topic-x)
    Manifest {
        tree: TreeMeta {
            name: "fixture".to_string(),
            created_at: "2026-05-19T13:00:00Z".to_string(),
            last_compiled_at: Some("2026-05-19T15:00:00Z".to_string()),
        },
        leaves: vec![
            LeafRecord {
                slug: "alpha".to_string(),
                file: "alpha.md".to_string(),
                title: "Alpha".to_string(),
                url: "https://example.com/a".to_string(),
                collected_at: "2026-05-19T14:00:00Z".to_string(),
                summary: None,
            },
            LeafRecord {
                slug: "beta".to_string(),
                file: "beta.md".to_string(),
                title: "Beta".to_string(),
                url: "https://example.com/b".to_string(),
                collected_at: "2026-05-19T14:30:00Z".to_string(),
                summary: None,
            },
            LeafRecord {
                slug: "gamma".to_string(),
                file: "gamma.md".to_string(),
                title: "Gamma".to_string(),
                url: "https://example.com/g".to_string(),
                collected_at: "2026-05-19T16:00:00Z".to_string(),
                summary: None,
            },
        ],
        branches: vec![
            BranchRecord {
                slug: "topic-x".to_string(),
                file: "branches/topic-x.md".to_string(),
                title: "Topic X".to_string(),
                created_at: "2026-05-19T15:00:00Z".to_string(),
                updated_at: "2026-05-19T15:00:00Z".to_string(),
                stale: false,
                leaves: vec!["alpha".to_string(), "beta".to_string()],
            },
            BranchRecord {
                slug: "topic-y".to_string(),
                file: "branches/topic-y.md".to_string(),
                title: "Topic Y".to_string(),
                created_at: "2026-05-19T15:00:00Z".to_string(),
                updated_at: "2026-05-19T15:00:00Z".to_string(),
                stale: false,
                leaves: vec!["beta".to_string()],
            },
        ],
    }
}

// ── resolution helpers ───────────────────────────────────────────────────────

#[test]
fn leaf_by_slug_returns_record_for_known_slug() {
    let m = resolution_fixture();
    let leaf = m.leaf_by_slug("beta").unwrap();
    assert_eq!(leaf.title, "Beta");
}

#[test]
fn leaf_by_slug_returns_none_for_unknown_slug() {
    let m = resolution_fixture();
    assert!(m.leaf_by_slug("nope").is_none());
}

#[test]
fn branch_by_slug_returns_record_for_known_slug() {
    let m = resolution_fixture();
    let b = m.branch_by_slug("topic-x").unwrap();
    assert_eq!(b.title, "Topic X");
}

#[test]
fn branch_by_slug_returns_none_for_unknown_slug() {
    let m = resolution_fixture();
    assert!(m.branch_by_slug("missing").is_none());
}

#[test]
fn uncompiled_leaves_returns_only_those_collected_after_last_compile() {
    let m = resolution_fixture();
    let uncompiled = m.uncompiled_leaves();
    assert_eq!(uncompiled.len(), 1);
    assert_eq!(uncompiled[0].slug, "gamma");
}

#[test]
fn uncompiled_leaves_returns_all_when_never_compiled() {
    let mut m = resolution_fixture();
    m.tree.last_compiled_at = None;
    let uncompiled = m.uncompiled_leaves();
    assert_eq!(uncompiled.len(), 3);
}

#[test]
fn uncompiled_leaves_empty_when_all_predate_last_compile() {
    let mut m = resolution_fixture();
    m.leaves.retain(|l| l.slug != "gamma");
    let uncompiled = m.uncompiled_leaves();
    assert!(uncompiled.is_empty());
}

#[test]
fn stale_branches_empty_when_no_branch_marked_stale() {
    let m = resolution_fixture();
    assert!(m.stale_branches().is_empty());
}

#[test]
fn stale_branches_returns_marked_records() {
    let mut m = resolution_fixture();
    m.branches[0].stale = true;
    let stale = m.stale_branches();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].slug, "topic-x");
}

#[test]
fn leaves_for_branch_resolves_slugs_to_records() {
    let m = resolution_fixture();
    let leaves = m.leaves_for_branch("topic-x");
    let slugs: Vec<&str> = leaves.iter().map(|l| l.slug.as_str()).collect();
    assert_eq!(slugs, vec!["alpha", "beta"]);
}

#[test]
fn leaves_for_branch_returns_empty_for_unknown_branch() {
    let m = resolution_fixture();
    assert!(m.leaves_for_branch("missing").is_empty());
}

#[test]
fn branches_for_leaf_returns_multiple_when_shared() {
    let m = resolution_fixture();
    let branches = m.branches_for_leaf("beta");
    let slugs: Vec<&str> = branches.iter().map(|b| b.slug.as_str()).collect();
    assert_eq!(slugs, vec!["topic-x", "topic-y"]);
}

#[test]
fn branches_for_leaf_returns_singleton_when_only_one_branch_owns_it() {
    let m = resolution_fixture();
    let branches = m.branches_for_leaf("alpha");
    let slugs: Vec<&str> = branches.iter().map(|b| b.slug.as_str()).collect();
    assert_eq!(slugs, vec!["topic-x"]);
}

#[test]
fn branches_for_leaf_returns_empty_for_unknown_leaf() {
    let m = resolution_fixture();
    assert!(m.branches_for_leaf("nope").is_empty());
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
