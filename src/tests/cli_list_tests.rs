use super::*;
use crate::domain::{Slug, Timestamp, Title, Url};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::time::SystemTime;
use tempfile::TempDir;

use crate::domain::manifest::{self, BranchRecord, LeafRecord, Manifest, TreeMeta};

#[derive(Debug, Clone, Eq, PartialEq)]
struct FileSnapshot {
    len: u64,
    modified: Option<SystemTime>,
    contents: String,
}

#[test]
fn empty_index_returns_empty_result() {
    let dir = TempDir::new().unwrap();
    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();
    assert!(result.leaves.is_empty());
    assert_eq!(result.total_index_entries, 0);
}

#[test]
fn default_order_follows_collection_order() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("second", "Second Leaf", "2025-01-02T00:00:00Z"),
            leaf("first", "First Leaf", "2025-01-01T00:00:00Z"),
            leaf("third", "Third Leaf", "2025-01-03T00:00:00Z"),
        ],
        &[],
    );
    write_leaf_files(dir.path(), &["second", "first", "third"]);

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();

    assert_eq!(
        files(&result.leaves),
        vec!["second.md", "first.md", "third.md"],
        "leaves preserve manifest insertion order"
    );
    assert_eq!(index_positions(&result.leaves), vec![0, 1, 2]);
}

#[test]
fn suspicious_path_is_degraded_and_never_read() {
    let sandbox = TempDir::new().unwrap();
    let tree_dir = sandbox.path().join("tree");
    fs::create_dir_all(&tree_dir).unwrap();
    // LeafRecord.file traverses out of the tree.
    write_manifest(
        &tree_dir,
        &[LeafRecord {
            slug: Slug::parse("outside").unwrap(),
            file: "../outside.md".to_string(),
            title: Title::new("Outside Title"),
            url: Url::parse("https://example.com/outside").unwrap(),
            collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            summary: None,
        }],
        &[],
    );
    fs::write(
        sandbox.path().join("outside.md"),
        "doesn't matter, list won't read me\n",
    )
    .unwrap();

    let result = list_leaves(&tree_dir, &ListOptions::default()).unwrap();
    let row = &result.leaves[0];

    assert_eq!(row.display_title, "Outside Title");
    assert!(row.degraded);
    assert_eq!(row.degradation_reasons, vec!["suspicious path"]);
}

#[test]
fn missing_file_yields_degraded_row() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[leaf("missing", "Index Title", "2025-01-01T00:00:00Z")],
        &[],
    );
    // Note: no leaf .md file written.

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();
    let row = &result.leaves[0];

    assert_eq!(row.file, "missing.md");
    assert_eq!(row.display_title, "Index Title");
    assert!(row.degraded);
    assert_eq!(row.degradation_reasons, vec!["missing file"]);
}

#[test]
fn display_title_falls_back_to_filename_when_manifest_title_empty() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("with-title", "Has Title", "2025-01-01T00:00:00Z"),
            leaf("filename-only", "", "2025-01-02T00:00:00Z"),
        ],
        &[],
    );
    write_leaf_files(dir.path(), &["with-title", "filename-only"]);

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();

    assert_eq!(result.leaves[0].display_title, "Has Title");
    assert_eq!(result.leaves[1].display_title, "filename-only");
}

#[test]
fn collected_at_is_taken_directly_from_manifest() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[leaf("only", "Only", "2025-06-01T10:00:00Z")],
        &[],
    );
    write_leaf_files(dir.path(), &["only"]);

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();

    assert_eq!(
        result.leaves[0].collected_at.as_deref(),
        Some("2025-06-01T10:00:00.000Z")
    );
    assert!(!result.leaves[0].degraded);
}

#[test]
fn branches_for_leaf_come_from_manifest_inverse() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("alpha", "Alpha", "2025-01-01T00:00:00Z"),
            leaf("beta", "Beta", "2025-01-01T00:00:00Z"),
            leaf("orphan", "Orphan", "2025-01-01T00:00:00Z"),
        ],
        &[("topic-x", &["alpha", "beta"]), ("topic-y", &["beta"])],
    );
    write_leaf_files(dir.path(), &["alpha", "beta", "orphan"]);

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();

    assert_eq!(result.leaves[0].branches, vec!["topic-x".to_string()]);
    assert_eq!(
        result.leaves[1].branches,
        vec!["topic-x".to_string(), "topic-y".to_string()]
    );
    assert!(result.leaves[2].branches.is_empty());
}

#[test]
fn branch_filter_is_exact() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("exact", "Exact", "2025-01-01T00:00:00Z"),
            leaf("partial", "Partial", "2025-01-01T00:00:00Z"),
            leaf("second-exact", "Second Exact", "2025-01-01T00:00:00Z"),
        ],
        &[
            ("rust", &["exact", "second-exact"]),
            ("rustacean", &["partial"]),
            ("systems", &["second-exact"]),
        ],
    );
    write_leaf_files(dir.path(), &["exact", "partial", "second-exact"]);

    let result = list_leaves(
        dir.path(),
        &ListOptions {
            branch: Some("rust".to_string()),
            ..ListOptions::default()
        },
    )
    .unwrap();

    assert_eq!(files(&result.leaves), vec!["exact.md", "second-exact.md"]);
}

#[test]
fn branch_filter_can_return_no_matches() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[leaf("only", "Only", "2025-01-01T00:00:00Z")],
        &[("rust", &["only"])],
    );
    write_leaf_files(dir.path(), &["only"]);

    let result = list_leaves(
        dir.path(),
        &ListOptions {
            branch: Some("missing".to_string()),
            ..ListOptions::default()
        },
    )
    .unwrap();

    assert!(result.leaves.is_empty());
    assert_eq!(result.total_index_entries, 1);
    assert_eq!(result.branch_filter.as_deref(), Some("missing"));
}

#[test]
fn recent_sorting_puts_newest_first_and_preserves_index_ties() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("old-a", "Old A", "2025-01-01T00:00:00Z"),
            leaf("middle", "Middle", "2025-01-15T00:00:00Z"),
            leaf("newest", "Newest", "2025-02-01T00:00:00Z"),
            leaf("old-b", "Old B", "2025-01-01T00:00:00Z"),
        ],
        &[],
    );
    write_leaf_files(dir.path(), &["old-a", "middle", "newest", "old-b"]);

    let result = list_leaves(
        dir.path(),
        &ListOptions {
            recent: true,
            ..ListOptions::default()
        },
    )
    .unwrap();

    // Newest first, then ties broken by index position (old-a before old-b)
    assert_eq!(
        files(&result.leaves),
        vec!["newest.md", "middle.md", "old-a.md", "old-b.md",]
    );
}

#[test]
fn limit_is_applied_after_filtering_and_sorting() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("mid", "Mid", "2025-01-02T00:00:00Z"),
            leaf("ignored", "Ignored", "2025-01-04T00:00:00Z"),
            leaf("newest", "Newest", "2025-01-03T00:00:00Z"),
            leaf("oldest", "Oldest", "2025-01-01T00:00:00Z"),
        ],
        &[
            ("keep", &["mid", "newest", "oldest"]),
            ("skip", &["ignored"]),
        ],
    );
    write_leaf_files(dir.path(), &["mid", "ignored", "newest", "oldest"]);

    let result = list_leaves(
        dir.path(),
        &ListOptions {
            branch: Some("keep".to_string()),
            recent: true,
            limit: Some(2),
        },
    )
    .unwrap();

    assert_eq!(files(&result.leaves), vec!["newest.md", "mid.md"]);
}

#[test]
fn list_leaves_is_read_only() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("one", "One", "2025-01-01T00:00:00Z"),
            LeafRecord {
                slug: Slug::parse("two").unwrap(),
                file: "nested/two.md".to_string(),
                title: Title::new("Two"),
                url: Url::parse("https://example.com/two").unwrap(),
                collected_at: Timestamp::parse("2025-01-02T00:00:00Z").unwrap(),
                summary: None,
            },
        ],
        &[("branch-a", &["one"])],
    );
    write_leaf_files(dir.path(), &["one"]);
    fs::create_dir_all(dir.path().join("nested")).unwrap();
    fs::write(dir.path().join("nested/two.md"), "body\n").unwrap();

    let before = snapshot_tree(dir.path());
    let _ = list_leaves(
        dir.path(),
        &ListOptions {
            recent: true,
            ..ListOptions::default()
        },
    )
    .unwrap();
    let after = snapshot_tree(dir.path());

    assert_eq!(before, after);
}

#[test]
fn render_human_formats_normal_rows() {
    let result = ListResult {
        leaves: vec![
            row(
                "alpha.md",
                "Alpha",
                Some("2025-06-01T10:00:00.000Z"),
                &["branch-a", "branch-b"],
                false,
                &[],
                0,
            ),
            row("beta.md", "Beta", None, &[], false, &[], 1),
        ],
        total_index_entries: 2,
        branch_filter: None,
    };

    assert_eq!(
        render_human(&result),
        "Alpha | 2025-06-01T10:00:00.000Z | [branch-a, branch-b]\nBeta | - | []\n"
    );
}

#[test]
fn render_human_empty_tree_message_is_clear() {
    let result = ListResult {
        leaves: Vec::new(),
        total_index_entries: 0,
        branch_filter: None,
    };

    assert_eq!(render_human(&result), "no leaves collected yet\n");
}

#[test]
fn render_human_branch_no_match_message_is_clear() {
    let result = ListResult {
        leaves: Vec::new(),
        total_index_entries: 3,
        branch_filter: Some("rust".to_string()),
    };

    assert_eq!(render_human(&result), "no leaves matched branch 'rust'\n");
}

#[test]
fn render_human_marks_degraded_rows() {
    let result = ListResult {
        leaves: vec![row(
            "broken.md",
            "Broken",
            None,
            &[],
            true,
            &["missing file"],
            0,
        )],
        total_index_entries: 1,
        branch_filter: None,
    };

    let rendered = render_human(&result);
    assert!(rendered.contains("DEGRADED"));
    assert!(rendered.contains("missing file"));
    assert_eq!(rendered, "Broken | - | [] | ⚠ DEGRADED: missing file\n");
}

#[test]
fn render_json_is_pretty_parseable_and_omits_index_position() {
    let result = ListResult {
        leaves: vec![row(
            "alpha.md",
            "Alpha",
            Some("2025-06-01T10:00:00.000Z"),
            &["branch-a"],
            true,
            &["missing file"],
            7,
        )],
        total_index_entries: 1,
        branch_filter: Some("branch-a".to_string()),
    };

    let rendered = render_json(&result).unwrap();
    let parsed: JsonValue = serde_json::from_str(&rendered).unwrap();
    let row = &parsed["leaves"][0];

    assert!(rendered.contains('\n'));
    assert_eq!(row["file"], "alpha.md");
    assert_eq!(row["display_title"], "Alpha");
    assert_eq!(row["collected_at"], "2025-06-01T10:00:00.000Z");
    assert_eq!(row["branches"][0], "branch-a");
    assert_eq!(row["degraded"], true);
    assert_eq!(row["degradation_reasons"][0], "missing file");
    assert!(row.get("index_position").is_none());
    assert!(parsed.get("leaves").is_some());
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn leaf(slug: &str, title: &str, collected_at: &str) -> LeafRecord {
    LeafRecord {
        slug: Slug::parse(slug).unwrap(),
        file: format!("{}.md", slug),
        title: Title::new(title),
        url: Url::parse(&format!("https://example.com/{slug}")).unwrap(),
        collected_at: Timestamp::parse(collected_at).unwrap(),
        summary: None,
    }
}

fn write_manifest(tree_dir: &Path, leaves: &[LeafRecord], branches: &[(&str, &[&str])]) {
    fs::create_dir_all(tree_dir.join(".bo")).unwrap();
    let branch_records: Vec<BranchRecord> = branches
        .iter()
        .map(|(slug, leaf_slugs)| BranchRecord {
            slug: Slug::parse(slug).unwrap(),
            file: format!("branches/{}.md", slug),
            title: Title::new(slug),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            updated_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            stale: false,
            leaves: leaf_slugs.iter().map(|s| Slug::parse(s).unwrap()).collect(),
        })
        .collect();
    let m = Manifest {
        tree: TreeMeta {
            name: "test".to_string(),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            last_compiled_at: None,
        },
        leaves: leaves.to_vec(),
        branches: branch_records,
    };
    manifest::write(&tree_dir.join(".bo/manifest.json"), &m).unwrap();
}

fn write_leaf_files(tree_dir: &Path, slugs: &[&str]) {
    for slug in slugs {
        fs::write(
            tree_dir.join(format!("{}.md", slug)),
            "---\ntitle: x\n---\n\nbody\n",
        )
        .unwrap();
    }
}

fn files(rows: &[ListLeafRow]) -> Vec<&str> {
    rows.iter().map(|row| row.file.as_str()).collect()
}

fn index_positions(rows: &[ListLeafRow]) -> Vec<usize> {
    rows.iter().map(|row| row.index_position).collect()
}

fn row(
    file: &str,
    display_title: &str,
    collected_at: Option<&str>,
    branches: &[&str],
    degraded: bool,
    degradation_reasons: &[&str],
    index_position: usize,
) -> ListLeafRow {
    ListLeafRow {
        file: file.to_string(),
        display_title: display_title.to_string(),
        collected_at: collected_at.map(str::to_string),
        branches: branches.iter().map(|branch| branch.to_string()).collect(),
        degraded,
        degradation_reasons: degradation_reasons
            .iter()
            .map(|reason| reason.to_string())
            .collect(),
        index_position,
    }
}

fn snapshot_tree(root: &Path) -> BTreeMap<String, FileSnapshot> {
    let mut snapshot = BTreeMap::new();
    collect_snapshots(root, root, &mut snapshot);
    snapshot
}

fn collect_snapshots(root: &Path, dir: &Path, snapshot: &mut BTreeMap<String, FileSnapshot>) {
    let mut entries = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            collect_snapshots(root, &path, snapshot);
            continue;
        }

        let relative = path
            .strip_prefix(root)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let metadata = fs::metadata(&path).unwrap();
        snapshot.insert(
            relative,
            FileSnapshot {
                len: metadata.len(),
                modified: metadata.modified().ok(),
                contents: fs::read_to_string(&path).unwrap_or_default(),
            },
        );
    }
}
