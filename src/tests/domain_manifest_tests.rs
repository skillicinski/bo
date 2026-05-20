use super::*;
use crate::domain::tree::{Tree, TreeConfig};
use crate::domain::{Slug, Timestamp, Title, Url};
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

fn sample_manifest() -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: "rust-notes".to_string(),
            created_at: Timestamp::parse("2026-05-19T14:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2026-05-19T14:32:11.000Z").unwrap()),
        },
        leaves: vec![LeafRecord {
            slug: Slug::parse("ownership-and-borrowing").unwrap(),
            file: "ownership-and-borrowing.md".to_string(),
            title: Title::new("Ownership and Borrowing"),
            url: Url::parse("https://example.com/ownership").unwrap(),
            collected_at: Timestamp::parse("2026-05-19T14:05:32Z").unwrap(),
            summary: Some("Rust's ownership rules.".to_string()),
        }],
        branches: vec![BranchRecord {
            slug: Slug::parse("memory-safety").unwrap(),
            file: "branches/memory-safety.md".to_string(),
            title: Title::new("Memory Safety"),
            created_at: Timestamp::parse("2026-05-19T14:32:11.000Z").unwrap(),
            updated_at: Timestamp::parse("2026-05-19T14:32:11.000Z").unwrap(),
            stale: false,
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

fn resolution_fixture() -> Manifest {
    // last compile at T=15:00
    // alpha + beta collected before → "compiled"
    // gamma collected after  → "uncompiled"
    // topic-x: [alpha, beta], topic-y: [beta] (shared with topic-x)
    Manifest {
        tree: TreeMeta {
            name: "fixture".to_string(),
            created_at: Timestamp::parse("2026-05-19T13:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2026-05-19T15:00:00Z").unwrap()),
        },
        leaves: vec![
            LeafRecord {
                slug: Slug::parse("alpha").unwrap(),
                file: "alpha.md".to_string(),
                title: Title::new("Alpha"),
                url: Url::parse("https://example.com/a").unwrap(),
                collected_at: Timestamp::parse("2026-05-19T14:00:00Z").unwrap(),
                summary: None,
            },
            LeafRecord {
                slug: Slug::parse("beta").unwrap(),
                file: "beta.md".to_string(),
                title: Title::new("Beta"),
                url: Url::parse("https://example.com/b").unwrap(),
                collected_at: Timestamp::parse("2026-05-19T14:30:00Z").unwrap(),
                summary: None,
            },
            LeafRecord {
                slug: Slug::parse("gamma").unwrap(),
                file: "gamma.md".to_string(),
                title: Title::new("Gamma"),
                url: Url::parse("https://example.com/g").unwrap(),
                collected_at: Timestamp::parse("2026-05-19T16:00:00Z").unwrap(),
                summary: None,
            },
        ],
        branches: vec![
            BranchRecord {
                slug: Slug::parse("topic-x").unwrap(),
                file: "branches/topic-x.md".to_string(),
                title: Title::new("Topic X"),
                created_at: Timestamp::parse("2026-05-19T15:00:00Z").unwrap(),
                updated_at: Timestamp::parse("2026-05-19T15:00:00Z").unwrap(),
                stale: false,
                leaves: vec![Slug::parse("alpha").unwrap(), Slug::parse("beta").unwrap()],
            },
            BranchRecord {
                slug: Slug::parse("topic-y").unwrap(),
                file: "branches/topic-y.md".to_string(),
                title: Title::new("Topic Y"),
                created_at: Timestamp::parse("2026-05-19T15:00:00Z").unwrap(),
                updated_at: Timestamp::parse("2026-05-19T15:00:00Z").unwrap(),
                stale: false,
                leaves: vec![Slug::parse("beta").unwrap()],
            },
        ],
    }
}

// ── resolution helpers ───────────────────────────────────────────────────────

#[test]
fn leaf_by_slug_returns_record_for_known_slug() {
    let m = resolution_fixture();
    let leaf = m.leaf_by_slug_str("beta").unwrap();
    assert_eq!(leaf.title.as_str(), "Beta");
}

#[test]
fn leaf_by_slug_returns_none_for_unknown_slug() {
    let m = resolution_fixture();
    assert!(m.leaf_by_slug_str("nope").is_none());
}

#[test]
fn branch_by_slug_returns_record_for_known_slug() {
    let m = resolution_fixture();
    let b = m.branch_by_slug_str("topic-x").unwrap();
    assert_eq!(b.title.as_str(), "Topic X");
}

#[test]
fn branch_by_slug_returns_none_for_unknown_slug() {
    let m = resolution_fixture();
    assert!(m.branch_by_slug_str("missing").is_none());
}

#[test]
fn uncompiled_leaves_returns_only_those_collected_after_last_compile() {
    let m = resolution_fixture();
    let uncompiled = m.uncompiled_leaves();
    assert_eq!(uncompiled.len(), 1);
    assert_eq!(uncompiled[0].slug.as_str(), "gamma");
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
    m.leaves.retain(|l| l.slug.as_str() != "gamma");
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
    assert_eq!(stale[0].slug.as_str(), "topic-x");
}

#[test]
fn leaves_for_branch_resolves_slugs_to_records() {
    let m = resolution_fixture();
    let leaves = m.leaves_for_branch_str("topic-x");
    let slugs: Vec<&str> = leaves.iter().map(|l| l.slug.as_str()).collect();
    assert_eq!(slugs, vec!["alpha", "beta"]);
}

#[test]
fn leaves_for_branch_returns_empty_for_unknown_branch() {
    let m = resolution_fixture();
    assert!(m.leaves_for_branch_str("missing").is_empty());
}

#[test]
fn branches_for_leaf_returns_multiple_when_shared() {
    let m = resolution_fixture();
    let branches = m.branches_for_leaf_str("beta");
    let slugs: Vec<&str> = branches.iter().map(|b| b.slug.as_str()).collect();
    assert_eq!(slugs, vec!["topic-x", "topic-y"]);
}

#[test]
fn branches_for_leaf_returns_singleton_when_only_one_branch_owns_it() {
    let m = resolution_fixture();
    let branches = m.branches_for_leaf_str("alpha");
    let slugs: Vec<&str> = branches.iter().map(|b| b.slug.as_str()).collect();
    assert_eq!(slugs, vec!["topic-x"]);
}

#[test]
fn branches_for_leaf_returns_empty_for_unknown_leaf() {
    let m = resolution_fixture();
    assert!(m.branches_for_leaf_str("nope").is_empty());
}

// ── reconstruction (T2.1) ───────────────────────────────────────────────────────

fn write_secondary_tree(root: &Path) {
    let bo_dir = root.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();

    // Three leaves: alpha, beta, gamma. alpha+beta in branch topic-x;
    // beta also in branch topic-y; gamma uncompiled.
    let index = r#"{"file":"alpha.md","title":"Alpha","url":"https://example.com/a"}
{"file":"beta.md","title":"Beta","url":"https://example.com/b"}
{"file":"gamma.md","title":"Gamma","url":"https://example.com/g"}
"#;
    fs::write(bo_dir.join("index.jsonl"), index).unwrap();

    let leaf = |slug: &str, title: &str, url: &str, collected_at: &str, summary: Option<&str>| {
        let mut s = String::new();
        s.push_str("---\n");
        s.push_str(&format!("title: \"{title}\"\n"));
        s.push_str(&format!("url: {url}\n"));
        s.push_str(&format!("collected_at: {collected_at}\n"));
        s.push_str(&format!("updated_at: {collected_at}\n"));
        if let Some(sum) = summary {
            s.push_str(&format!("summary: \"{sum}\"\n"));
        }
        s.push_str("---\n\n# ");
        s.push_str(title);
        s.push_str("\n\nbody.\n");
        fs::write(root.join(format!("{slug}.md")), s).unwrap();
    };
    leaf(
        "alpha",
        "Alpha",
        "https://example.com/a",
        "2026-05-19T14:00:00Z",
        Some("sum-alpha"),
    );
    leaf(
        "beta",
        "Beta",
        "https://example.com/b",
        "2026-05-19T14:30:00Z",
        None,
    );
    leaf(
        "gamma",
        "Gamma",
        "https://example.com/g",
        "2026-05-19T16:00:00Z",
        Some("sum-gamma"),
    );

    // Two branches.
    let branches_dir = root.join("branches");
    fs::create_dir_all(&branches_dir).unwrap();
    let branch = |slug: &str, title: &str, created_at: &str, updated_at: &str, leaves: &[&str]| {
        let mut s = String::new();
        s.push_str("---\n");
        s.push_str(&format!("title: {title}\n"));
        s.push_str(&format!("created_at: {created_at}\n"));
        s.push_str(&format!("updated_at: {updated_at}\n"));
        s.push_str("leaves:\n");
        for l in leaves {
            s.push_str(&format!("- {l}\n"));
        }
        s.push_str("---\n\n# ");
        s.push_str(title);
        s.push_str("\n\nbody.\n");
        fs::write(branches_dir.join(format!("{slug}.md")), s).unwrap();
    };
    branch(
        "topic-x",
        "Topic X",
        "2026-05-19T15:00:00Z",
        "2026-05-19T15:00:00Z",
        &["alpha.md", "beta.md"],
    );
    branch(
        "topic-y",
        "Topic Y",
        "2026-05-19T15:30:00Z",
        "2026-05-19T15:30:00Z",
        &["beta.md"],
    );
}

fn fixture_tree(td: &TempDir) -> Tree {
    Tree::from_config(&TreeConfig {
        output_dir: PathBuf::from(td.path()),
        name: Some("fixture".to_string()),
        created_at: Some("2026-05-19T13:00:00Z".to_string()),
    })
}

#[test]
fn read_or_reconstruct_returns_existing_manifest_when_present() {
    let dir = TempDir::new().unwrap();
    let tree = fixture_tree(&dir);
    let original = sample_manifest();
    write(&tree.manifest_path(), &original).unwrap();

    let loaded = read_or_reconstruct(&tree).unwrap();

    assert_eq!(loaded, original);
}

#[test]
fn read_or_reconstruct_does_not_rebuild_from_secondary_when_manifest_absent() {
    let dir = TempDir::new().unwrap();
    write_secondary_tree(dir.path());
    let tree = fixture_tree(&dir);

    let err = read_or_reconstruct(&tree).unwrap_err();

    assert!(matches!(err, ManifestError::TreeNotInitialized));
    assert!(!tree.manifest_path().exists());
}

#[test]
fn read_or_reconstruct_does_not_persist_when_manifest_absent() {
    let dir = TempDir::new().unwrap();
    write_secondary_tree(dir.path());
    let tree = fixture_tree(&dir);

    assert!(!tree.manifest_path().exists());

    let err = read_or_reconstruct(&tree).unwrap_err();

    assert!(matches!(err, ManifestError::TreeNotInitialized));
    assert!(!tree.manifest_path().exists());
}

#[test]
fn read_or_reconstruct_returns_tree_not_initialized_when_secondary_also_empty() {
    let dir = TempDir::new().unwrap();
    let tree = fixture_tree(&dir);
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    let err = read_or_reconstruct(&tree).unwrap_err();
    assert!(
        matches!(err, ManifestError::TreeNotInitialized),
        "got: {err}"
    );
}

#[test]
fn read_or_reconstruct_propagates_parse_error_without_reconstructing() {
    let dir = TempDir::new().unwrap();
    write_secondary_tree(dir.path()); // ensure secondary exists
    let tree = fixture_tree(&dir);
    fs::create_dir_all(tree.manifest_path().parent().unwrap()).unwrap();
    fs::write(tree.manifest_path(), "{not valid").unwrap();

    let err = read_or_reconstruct(&tree).unwrap_err();
    assert!(matches!(err, ManifestError::Parse(_)), "got: {err}");
    // The corrupt file should still be on disk — we did not silently overwrite.
    let raw = fs::read_to_string(tree.manifest_path()).unwrap();
    assert_eq!(raw, "{not valid");
}

#[test]
fn missing_manifest_does_not_use_tree_name_fallbacks() {
    let dir = TempDir::new().unwrap();
    write_secondary_tree(dir.path());
    let tree = Tree::from_config(&TreeConfig {
        output_dir: PathBuf::from(dir.path()),
        name: None,
        created_at: Some("2026-05-19T13:00:00Z".to_string()),
    });

    let err = read_or_reconstruct(&tree).unwrap_err();
    assert!(matches!(err, ManifestError::TreeNotInitialized));
}

#[test]
fn missing_manifest_does_not_round_trip_secondary_store() {
    let dir = TempDir::new().unwrap();
    write_secondary_tree(dir.path());
    let tree = fixture_tree(&dir);

    let err = read_or_reconstruct(&tree).unwrap_err();
    assert!(matches!(err, ManifestError::TreeNotInitialized));
    assert!(read(&tree.manifest_path()).is_err());
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
