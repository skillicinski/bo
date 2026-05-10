use super::*;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::time::SystemTime;
use tempfile::TempDir;

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
fn default_order_follows_index_order() {
    let dir = TempDir::new().unwrap();
    write_index(
        dir.path(),
        &[
            ("second.md", "Second Index Title"),
            ("first.md", "First Index Title"),
            ("third.md", "Third Index Title"),
        ],
    );
    write_leaf(
        dir.path(),
        "second.md",
        "title: Second Leaf\ncollected_at: 2025-01-02T00:00:00Z\n",
    );
    write_leaf(
        dir.path(),
        "first.md",
        "title: First Leaf\ncollected_at: 2025-01-01T00:00:00Z\n",
    );
    write_leaf(
        dir.path(),
        "third.md",
        "title: Third Leaf\ncollected_at: 2025-01-03T00:00:00Z\n",
    );

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();

    assert_eq!(result.total_index_entries, 3);
    assert_eq!(
        files(&result.leaves),
        vec!["second.md", "first.md", "third.md"]
    );
    assert_eq!(index_positions(&result.leaves), vec![0, 1, 2]);
}

#[test]
fn suspicious_path_is_degraded_and_never_read() {
    let sandbox = TempDir::new().unwrap();
    let tree_dir = sandbox.path().join("tree");
    fs::create_dir_all(&tree_dir).unwrap();
    write_index(&tree_dir, &[("../outside.md", "Index Title")]);
    fs::write(
        sandbox.path().join("outside.md"),
        "---\ntitle: Outside Title\ncollected_at: 2025-01-01T00:00:00Z\n---\n\noutside\n",
    )
    .unwrap();

    let result = list_leaves(&tree_dir, &ListOptions::default()).unwrap();
    let row = &result.leaves[0];

    assert_eq!(row.display_title, "Index Title");
    assert!(row.degraded);
    assert_eq!(row.degradation_reasons, vec!["suspicious path"]);
    assert!(row.collected_at.is_none());
    assert!(row.branches.is_empty());
}

#[test]
fn missing_file_yields_degraded_row() {
    let dir = TempDir::new().unwrap();
    write_index(dir.path(), &[("missing.md", "Index Title")]);

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();
    let row = &result.leaves[0];

    assert_eq!(row.file, "missing.md");
    assert_eq!(row.display_title, "Index Title");
    assert!(row.degraded);
    assert_eq!(row.degradation_reasons, vec!["missing file"]);
}

#[test]
fn invalid_frontmatter_yields_degraded_row_with_fallback_title() {
    let dir = TempDir::new().unwrap();
    write_index(dir.path(), &[("broken.md", "Index Title")]);
    write_raw_file(
        dir.path(),
        "broken.md",
        "---\n: invalid: yaml\n---\n\nbody\n",
    );

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();
    let row = &result.leaves[0];

    assert_eq!(row.display_title, "Index Title");
    assert!(row.degraded);
    assert_eq!(row.degradation_reasons, vec!["invalid frontmatter"]);
    assert!(row.collected_at.is_none());
    assert!(row.branches.is_empty());
}

#[test]
fn display_title_falls_back_leaf_then_index_then_filename() {
    let dir = TempDir::new().unwrap();
    write_index(
        dir.path(),
        &[
            ("leaf-title.md", "Index Title 1"),
            ("index-title.md", "Index Title 2"),
            ("filename-only.md", ""),
        ],
    );
    write_leaf(
        dir.path(),
        "leaf-title.md",
        "title: Leaf Title\ncollected_at: 2025-01-01T00:00:00Z\n",
    );
    write_leaf(
        dir.path(),
        "index-title.md",
        "title: \"\"\ncollected_at: 2025-01-02T00:00:00Z\n",
    );
    write_leaf(
        dir.path(),
        "filename-only.md",
        "title: \"\"\ncollected_at: 2025-01-03T00:00:00Z\n",
    );

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();

    assert_eq!(result.leaves[0].display_title, "Leaf Title");
    assert_eq!(result.leaves[1].display_title, "Index Title 2");
    assert_eq!(result.leaves[2].display_title, "filename-only");
}

#[test]
fn collected_at_valid_missing_and_invalid_are_handled() {
    let dir = TempDir::new().unwrap();
    write_index(
        dir.path(),
        &[
            ("valid.md", "Valid"),
            ("missing.md", "Missing"),
            ("invalid.md", "Invalid"),
        ],
    );
    write_leaf(
        dir.path(),
        "valid.md",
        "title: Valid\ncollected_at: 2025-06-01T10:00:00Z\n",
    );
    write_leaf(dir.path(), "missing.md", "title: Missing\n");
    write_leaf(
        dir.path(),
        "invalid.md",
        "title: Invalid\ncollected_at: not-a-date\n",
    );

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();

    assert_eq!(
        result.leaves[0].collected_at.as_deref(),
        Some("2025-06-01T10:00:00Z")
    );
    assert!(!result.leaves[0].degraded);

    assert!(result.leaves[1].collected_at.is_none());
    assert!(result.leaves[1].degraded);
    assert_eq!(
        result.leaves[1].degradation_reasons,
        vec!["missing collected_at"]
    );

    assert!(result.leaves[2].collected_at.is_none());
    assert!(result.leaves[2].degraded);
    assert_eq!(
        result.leaves[2].degradation_reasons,
        vec!["invalid collected_at"]
    );
}

#[test]
fn branches_are_normalized_and_invalid_shapes_degrade() {
    let dir = TempDir::new().unwrap();
    write_index(
        dir.path(),
        &[
            ("missing-branches.md", "Missing Branches"),
            ("empty-branches.md", "Empty Branches"),
            ("string-branches.md", "String Branches"),
            ("mixed-branches.md", "Mixed Branches"),
            ("scalar-branches.md", "Scalar Branches"),
        ],
    );
    write_leaf(
        dir.path(),
        "missing-branches.md",
        "title: Missing Branches\ncollected_at: 2025-01-01T00:00:00Z\n",
    );
    write_leaf(
        dir.path(),
        "empty-branches.md",
        "title: Empty Branches\ncollected_at: 2025-01-01T00:00:00Z\nbranches: []\n",
    );
    write_leaf(
        dir.path(),
        "string-branches.md",
        "title: String Branches\ncollected_at: 2025-01-01T00:00:00Z\nbranches:\n  - branch_a\n  - branch_b\n",
    );
    write_leaf(
        dir.path(),
        "mixed-branches.md",
        "title: Mixed Branches\ncollected_at: 2025-01-01T00:00:00Z\nbranches:\n  - branch_a\n  - 7\n  - branch_b\n",
    );
    write_leaf(
        dir.path(),
        "scalar-branches.md",
        "title: Scalar Branches\ncollected_at: 2025-01-01T00:00:00Z\nbranches: nope\n",
    );

    let result = list_leaves(dir.path(), &ListOptions::default()).unwrap();

    assert!(result.leaves[0].branches.is_empty());
    assert!(!result.leaves[0].degraded);

    assert!(result.leaves[1].branches.is_empty());
    assert!(!result.leaves[1].degraded);

    assert_eq!(
        result.leaves[2].branches,
        vec!["branch_a".to_string(), "branch_b".to_string()]
    );
    assert!(!result.leaves[2].degraded);

    assert_eq!(
        result.leaves[3].branches,
        vec!["branch_a".to_string(), "branch_b".to_string()]
    );
    assert!(result.leaves[3].degraded);
    assert_eq!(
        result.leaves[3].degradation_reasons,
        vec!["invalid branches"]
    );

    assert!(result.leaves[4].branches.is_empty());
    assert!(result.leaves[4].degraded);
    assert_eq!(
        result.leaves[4].degradation_reasons,
        vec!["invalid branches"]
    );
}

#[test]
fn branch_filter_is_exact() {
    let dir = TempDir::new().unwrap();
    write_index(
        dir.path(),
        &[
            ("exact.md", "Exact"),
            ("partial.md", "Partial"),
            ("second-exact.md", "Second Exact"),
        ],
    );
    write_leaf(
        dir.path(),
        "exact.md",
        "title: Exact\ncollected_at: 2025-01-01T00:00:00Z\nbranches:\n  - rust\n",
    );
    write_leaf(
        dir.path(),
        "partial.md",
        "title: Partial\ncollected_at: 2025-01-01T00:00:00Z\nbranches:\n  - rustacean\n",
    );
    write_leaf(
        dir.path(),
        "second-exact.md",
        "title: Second Exact\ncollected_at: 2025-01-01T00:00:00Z\nbranches:\n  - systems\n  - rust\n",
    );

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
    write_index(dir.path(), &[("only.md", "Only")]);
    write_leaf(
        dir.path(),
        "only.md",
        "title: Only\ncollected_at: 2025-01-01T00:00:00Z\nbranches:\n  - rust\n",
    );

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
fn recent_sorting_puts_valid_dates_first_and_preserves_index_ties() {
    let dir = TempDir::new().unwrap();
    write_index(
        dir.path(),
        &[
            ("old-a.md", "Old A"),
            ("missing.md", "Missing"),
            ("newest.md", "Newest"),
            ("invalid.md", "Invalid"),
            ("old-b.md", "Old B"),
        ],
    );
    write_leaf(
        dir.path(),
        "old-a.md",
        "title: Old A\ncollected_at: 2025-01-01T00:00:00Z\n",
    );
    write_leaf(dir.path(), "missing.md", "title: Missing\n");
    write_leaf(
        dir.path(),
        "newest.md",
        "title: Newest\ncollected_at: 2025-02-01T00:00:00Z\n",
    );
    write_leaf(
        dir.path(),
        "invalid.md",
        "title: Invalid\ncollected_at: not-a-date\n",
    );
    write_leaf(
        dir.path(),
        "old-b.md",
        "title: Old B\ncollected_at: 2025-01-01T00:00:00Z\n",
    );

    let result = list_leaves(
        dir.path(),
        &ListOptions {
            recent: true,
            ..ListOptions::default()
        },
    )
    .unwrap();

    assert_eq!(
        files(&result.leaves),
        vec![
            "newest.md",
            "old-a.md",
            "old-b.md",
            "missing.md",
            "invalid.md"
        ]
    );
}

#[test]
fn limit_is_applied_after_filtering_and_sorting() {
    let dir = TempDir::new().unwrap();
    write_index(
        dir.path(),
        &[
            ("mid.md", "Mid"),
            ("ignored.md", "Ignored"),
            ("newest.md", "Newest"),
            ("oldest.md", "Oldest"),
        ],
    );
    write_leaf(
        dir.path(),
        "mid.md",
        "title: Mid\ncollected_at: 2025-01-02T00:00:00Z\nbranches:\n  - keep\n",
    );
    write_leaf(
        dir.path(),
        "ignored.md",
        "title: Ignored\ncollected_at: 2025-01-04T00:00:00Z\nbranches:\n  - skip\n",
    );
    write_leaf(
        dir.path(),
        "newest.md",
        "title: Newest\ncollected_at: 2025-01-03T00:00:00Z\nbranches:\n  - keep\n",
    );
    write_leaf(
        dir.path(),
        "oldest.md",
        "title: Oldest\ncollected_at: 2025-01-01T00:00:00Z\nbranches:\n  - keep\n",
    );

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
    write_index(dir.path(), &[("one.md", "One"), ("nested/two.md", "Two")]);
    write_leaf(
        dir.path(),
        "one.md",
        "title: One\ncollected_at: 2025-01-01T00:00:00Z\nbranches:\n  - branch_a\n",
    );
    write_leaf(
        dir.path(),
        "nested/two.md",
        "title: Two\ncollected_at: 2025-01-02T00:00:00Z\nbranches: []\n",
    );

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
                Some("2025-06-01T10:00:00Z"),
                &["branch_a", "branch_b"],
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
        "Alpha | 2025-06-01T10:00:00Z | [branch_a, branch_b]\nBeta | - | []\n"
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
            Some("2025-06-01T10:00:00Z"),
            &["branch_a"],
            true,
            &["invalid branches"],
            7,
        )],
        total_index_entries: 1,
        branch_filter: Some("branch_a".to_string()),
    };

    let rendered = render_json(&result).unwrap();
    let parsed: JsonValue = serde_json::from_str(&rendered).unwrap();
    let row = &parsed["leaves"][0];

    assert!(rendered.contains('\n'));
    assert_eq!(row["file"], "alpha.md");
    assert_eq!(row["display_title"], "Alpha");
    assert_eq!(row["collected_at"], "2025-06-01T10:00:00Z");
    assert_eq!(row["branches"][0], "branch_a");
    assert_eq!(row["degraded"], true);
    assert_eq!(row["degradation_reasons"][0], "invalid branches");
    assert!(row.get("index_position").is_none());
    assert!(parsed.get("leaves").is_some());
}

fn write_index(tree_dir: &Path, entries: &[(&str, &str)]) {
    let lines = entries
        .iter()
        .map(|(file, title)| {
            serde_json::json!({
                "file": file,
                "title": title,
                "url": format!("https://example.com/{file}"),
            })
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");

    fs::write(tree_dir.join("index.jsonl"), format!("{lines}\n")).unwrap();
}

fn write_leaf(tree_dir: &Path, relative_path: &str, yaml_fields: &str) {
    write_raw_file(
        tree_dir,
        relative_path,
        &format!("---\n{yaml_fields}---\n\nbody\n"),
    );
}

fn write_raw_file(tree_dir: &Path, relative_path: &str, contents: &str) {
    let path = tree_dir.join(relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, contents).unwrap();
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
                contents: fs::read_to_string(&path).unwrap(),
            },
        );
    }
}
