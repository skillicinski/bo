use super::*;
use crate::domain::{Slug, Timestamp};
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
    let result = list_tree(dir.path(), &ListOptions::default()).unwrap();
    assert!(
        matches!(result.view, ListView::BranchCentric { ref branches, ref unbranched } if branches.is_empty() && unbranched.is_empty())
    );
    assert_eq!(result.total_branches, 0);
    assert_eq!(result.total_leaves, 0);
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

    let result = list_tree(dir.path(), &leaves_options()).unwrap();

    let leaves = leaves_from_result(&result);
    assert_eq!(
        files(&leaves),
        vec!["second.md", "first.md", "third.md"],
        "leaves preserve manifest insertion order"
    );
    assert_eq!(index_positions(&leaves), vec![0, 1, 2]);
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
            title: ("Outside Title").to_string(),
            url: ("https://example.com/outside").to_string(),
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

    let result = list_tree(&tree_dir, &leaves_options()).unwrap();
    let leaves = leaves_from_result(&result);
    let row = &leaves[0];

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

    let result = list_tree(dir.path(), &leaves_options()).unwrap();
    let leaves = leaves_from_result(&result);
    let row = &leaves[0];

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

    let result = list_tree(dir.path(), &leaves_options()).unwrap();
    let leaves = leaves_from_result(&result);

    assert_eq!(leaves[0].display_title, "Has Title");
    assert_eq!(leaves[1].display_title, "filename-only");
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

    let result = list_tree(dir.path(), &leaves_options()).unwrap();
    let leaves = leaves_from_result(&result);

    assert_eq!(
        leaves[0].collected_at.as_deref(),
        Some("2025-06-01T10:00:00.000Z")
    );
    assert!(!leaves[0].degraded);
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

    let result = list_tree(dir.path(), &leaves_options()).unwrap();
    let leaves = leaves_from_result(&result);

    assert_eq!(leaves[0].branches, vec!["topic-x".to_string()]);
    assert_eq!(leaves[0].branch_count, 1);
    assert_eq!(
        leaves[1].branches,
        vec!["topic-x".to_string(), "topic-y".to_string()]
    );
    assert_eq!(leaves[1].branch_count, 2);
    assert!(leaves[2].branches.is_empty());
    assert_eq!(leaves[2].branch_count, 0);
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

    let result = list_tree(
        dir.path(),
        &ListOptions {
            view: ListViewMode::Leaves,
            terms: Vec::new(),
            branch: Some("rust".to_string()),
            ..ListOptions::default()
        },
    )
    .unwrap();

    let leaves = leaves_from_result(&result);
    assert_eq!(files(&leaves), vec!["exact.md", "second-exact.md"]);
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

    let result = list_tree(
        dir.path(),
        &ListOptions {
            view: ListViewMode::Leaves,
            terms: Vec::new(),
            branch: Some("missing".to_string()),
            ..ListOptions::default()
        },
    )
    .unwrap();

    let leaves = leaves_from_result(&result);
    assert!(leaves.is_empty());
    assert_eq!(result.total_leaves, 1);
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

    let result = list_tree(
        dir.path(),
        &ListOptions {
            view: ListViewMode::Leaves,
            terms: Vec::new(),
            recent: true,
            ..ListOptions::default()
        },
    )
    .unwrap();

    let leaves = leaves_from_result(&result);
    // Newest first, then ties broken by index position (old-a before old-b)
    assert_eq!(
        files(&leaves),
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

    let result = list_tree(
        dir.path(),
        &ListOptions {
            view: ListViewMode::Leaves,
            terms: Vec::new(),
            branch: Some("keep".to_string()),
            recent: true,
            limit: Some(2),
        },
    )
    .unwrap();

    let leaves = leaves_from_result(&result);
    assert_eq!(files(&leaves), vec!["newest.md", "mid.md"]);
}

#[test]
fn list_tree_is_read_only() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("one", "One", "2025-01-01T00:00:00Z"),
            LeafRecord {
                slug: Slug::parse("two").unwrap(),
                file: "nested/two.md".to_string(),
                title: ("Two").to_string(),
                url: ("https://example.com/two").to_string(),
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
    let _ = list_tree(
        dir.path(),
        &ListOptions {
            view: ListViewMode::Leaves,
            ..ListOptions::default()
        },
    )
    .unwrap();
    let after = snapshot_tree(dir.path());

    assert_eq!(before, after);
}

// ── render tests ────────────────────────────────────────────────────────────

#[test]
fn render_human_branch_centric_formats_nested_leaves() {
    let result = ListResult {
        view: ListView::BranchCentric {
            branches: vec![BranchWithLeaves {
                slug: "topic-x".to_string(),
                title: "Topic X".to_string(),
                updated_at: Some("2025-01-01T00:00:00.000Z".to_string()),
                leaves: vec![leaf_row(
                    "alpha",
                    "alpha.md",
                    "Alpha",
                    Some("2025-06-01T10:00:00.000Z"),
                    &["topic-x"],
                    false,
                    &[],
                    0,
                )],
            }],
            unbranched: vec![leaf_row(
                "orphan",
                "orphan.md",
                "Orphan",
                None,
                &[],
                false,
                &[],
                1,
            )],
        },
        total_branches: 1,
        total_leaves: 2,
        branch_filter: None,
    };

    let rendered = render_human(&result);
    assert!(rendered.contains("## Topic X"), "{rendered}");
    assert!(rendered.contains("Alpha"), "{rendered}");
    assert!(rendered.contains("## unbranched"), "{rendered}");
    assert!(rendered.contains("Orphan"), "{rendered}");
}

#[test]
fn render_human_branches_view() {
    let result = ListResult {
        view: ListView::Branches {
            items: vec![
                BranchRow {
                    slug: "topic-x".to_string(),
                    title: "Topic X".to_string(),
                    leaf_count: 3,
                    updated_at: Some("2025-01-01T00:00:00.000Z".to_string()),
                },
                BranchRow {
                    slug: "topic-y".to_string(),
                    title: "Topic Y".to_string(),
                    leaf_count: 1,
                    updated_at: Some("2025-01-02T00:00:00.000Z".to_string()),
                },
            ],
        },
        total_branches: 2,
        total_leaves: 4,
        branch_filter: None,
    };

    let rendered = render_human(&result);
    assert!(
        rendered.contains("topic-x | Topic X | 3 leaves"),
        "{rendered}"
    );
    assert!(
        rendered.contains("topic-y | Topic Y | 1 leaves"),
        "{rendered}"
    );
}

#[test]
fn render_human_leaves_view_formats_rows() {
    let result = ListResult {
        view: ListView::Leaves {
            items: vec![
                leaf_row(
                    "alpha",
                    "alpha.md",
                    "Alpha",
                    Some("2025-06-01T10:00:00.000Z"),
                    &["branch-a", "branch-b"],
                    false,
                    &[],
                    0,
                ),
                leaf_row("beta", "beta.md", "Beta", None, &[], false, &[], 1),
            ],
        },
        total_branches: 2,
        total_leaves: 2,
        branch_filter: None,
    };

    let rendered = render_human(&result);
    assert!(
        rendered.contains("Alpha | alpha | 2025-06-01T10:00:00.000Z | 2 branches"),
        "{rendered}"
    );
    assert!(
        rendered.contains("Beta | beta | - | 0 branches"),
        "{rendered}"
    );
}

#[test]
fn render_human_leaves_marks_degraded_rows() {
    let result = ListResult {
        view: ListView::Leaves {
            items: vec![leaf_row(
                "broken",
                "broken.md",
                "Broken",
                None,
                &[],
                true,
                &["missing file"],
                0,
            )],
        },
        total_branches: 0,
        total_leaves: 1,
        branch_filter: None,
    };

    let rendered = render_human(&result);
    assert!(rendered.contains("DEGRADED"));
    assert!(rendered.contains("missing file"));
}

#[test]
fn render_human_empty_tree_message_is_clear() {
    let result = ListResult {
        view: ListView::BranchCentric {
            branches: Vec::new(),
            unbranched: Vec::new(),
        },
        total_branches: 0,
        total_leaves: 0,
        branch_filter: None,
    };

    assert_eq!(render_human(&result), "no content in tree\n");
}

#[test]
fn render_human_leaves_branch_no_match_message_is_clear() {
    let result = ListResult {
        view: ListView::Leaves { items: Vec::new() },
        total_branches: 1,
        total_leaves: 3,
        branch_filter: Some("rust".to_string()),
    };

    assert_eq!(render_human(&result), "no leaves matched branch 'rust'\n");
}

#[test]
fn render_human_no_branches_message() {
    let result = ListResult {
        view: ListView::Branches { items: Vec::new() },
        total_branches: 0,
        total_leaves: 5,
        branch_filter: None,
    };

    assert_eq!(render_human(&result), "no branches compiled yet\n");
}

#[test]
fn render_human_branch_centric_no_branches_no_matches_message() {
    let result = ListResult {
        view: ListView::BranchCentric {
            branches: Vec::new(),
            unbranched: Vec::new(),
        },
        total_branches: 2,
        total_leaves: 5,
        branch_filter: Some("rust".to_string()),
    };

    assert_eq!(render_human(&result), "no branches matched 'rust'\n");
}

#[test]
fn terms_filters_leaves_by_title_and_slug() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("rust-basics", "Rust Basics", "2025-01-01T00:00:00Z"),
            leaf("go-intro", "Go Introduction", "2025-01-02T00:00:00Z"),
            leaf("rust-advanced", "Advanced Rust", "2025-01-03T00:00:00Z"),
        ],
        &[],
    );
    write_leaf_files(dir.path(), &["rust-basics", "go-intro", "rust-advanced"]);

    let result = list_tree(
        dir.path(),
        &ListOptions {
            view: ListViewMode::Leaves,
            terms: vec!["rust".to_string()],
            ..ListOptions::default()
        },
    )
    .unwrap();

    let leaves = leaves_from_result(&result);
    assert_eq!(leaves.len(), 2);
    assert_eq!(leaves[0].display_title, "Rust Basics");
    assert_eq!(leaves[1].display_title, "Advanced Rust");
}

#[test]
fn terms_filters_branches_by_title_and_slug() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("a", "A", "2025-01-01T00:00:00Z"),
            leaf("b", "B", "2025-01-01T00:00:00Z"),
            leaf("c", "C", "2025-01-01T00:00:00Z"),
        ],
        &[
            ("rust-basics", &["a"]),
            ("go-intro", &["b"]),
            ("rust-advanced", &["c"]),
        ],
    );
    write_leaf_files(dir.path(), &["a", "b", "c"]);

    let result = list_tree(
        dir.path(),
        &ListOptions {
            view: ListViewMode::Branches,
            terms: vec!["rust".to_string()],
            ..ListOptions::default()
        },
    )
    .unwrap();

    if let ListView::Branches { items: rows } = &result.view {
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].slug, "rust-basics");
        assert_eq!(rows[1].slug, "rust-advanced");
    } else {
        panic!("expected Branches view");
    }
}

#[test]
fn terms_filters_branch_centric_by_branch_and_leaf_match() {
    let dir = TempDir::new().unwrap();
    write_manifest(
        dir.path(),
        &[
            leaf("rust-leaf", "Rust Leaf", "2025-01-01T00:00:00Z"),
            leaf("go-leaf", "Go Leaf", "2025-01-01T00:00:00Z"),
        ],
        &[("rust-branch", &["rust-leaf"]), ("go-branch", &["go-leaf"])],
    );
    write_leaf_files(dir.path(), &["rust-leaf", "go-leaf"]);

    let result = list_tree(
        dir.path(),
        &ListOptions {
            view: ListViewMode::BranchCentric,
            terms: vec!["rust".to_string()],
            ..ListOptions::default()
        },
    )
    .unwrap();

    if let ListView::BranchCentric {
        branches,
        unbranched,
    } = &result.view
    {
        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].slug, "rust-branch");
        assert_eq!(branches[0].leaves.len(), 1);
        assert_eq!(branches[0].leaves[0].slug, "rust-leaf");
        assert!(unbranched.is_empty());
    } else {
        panic!("expected BranchCentric view");
    }
}

#[test]
fn degraded_leaves_collects_from_branch_centric() {
    let result = ListResult {
        view: ListView::BranchCentric {
            branches: vec![BranchWithLeaves {
                slug: "topic-x".to_string(),
                title: "Topic X".to_string(),
                updated_at: Some("2025-01-01T00:00:00.000Z".to_string()),
                leaves: vec![leaf_row(
                    "alpha",
                    "alpha.md",
                    "Alpha",
                    None,
                    &[],
                    true,
                    &["missing file"],
                    0,
                )],
            }],
            unbranched: vec![leaf_row(
                "orphan",
                "orphan.md",
                "Orphan",
                None,
                &[],
                true,
                &["suspicious path"],
                1,
            )],
        },
        total_branches: 1,
        total_leaves: 2,
        branch_filter: None,
    };

    let degraded = result.degraded_leaves();
    assert_eq!(degraded.len(), 2);
    assert_eq!(degraded[0].slug, "alpha");
    assert_eq!(degraded[1].slug, "orphan");
}

#[test]
fn degraded_leaves_empty_for_branches_view() {
    let result = ListResult {
        view: ListView::Branches {
            items: vec![BranchRow {
                slug: "topic-x".to_string(),
                title: "Topic X".to_string(),
                leaf_count: 1,
                updated_at: Some("2025-01-01T00:00:00.000Z".to_string()),
            }],
        },
        total_branches: 1,
        total_leaves: 1,
        branch_filter: None,
    };

    assert!(result.degraded_leaves().is_empty());
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn leaf(slug: &str, title: &str, collected_at: &str) -> LeafRecord {
    LeafRecord {
        slug: Slug::parse(slug).unwrap(),
        file: format!("{}.md", slug),
        title: title.to_string(),
        url: format!("https://example.com/{slug}"),
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
            title: slug.to_string(),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            updated_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
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

fn leaves_options() -> ListOptions {
    ListOptions {
        view: ListViewMode::Leaves,
        terms: Vec::new(),
        ..ListOptions::default()
    }
}

fn leaves_from_result(result: &ListResult) -> Vec<ListLeafRow> {
    match &result.view {
        ListView::Leaves { items: rows } => rows.clone(),
        _ => panic!("expected Leaves view"),
    }
}

#[allow(clippy::too_many_arguments)]
fn leaf_row(
    slug: &str,
    file: &str,
    display_title: &str,
    collected_at: Option<&str>,
    branches: &[&str],
    degraded: bool,
    degradation_reasons: &[&str],
    index_position: usize,
) -> ListLeafRow {
    let branch_slugs: Vec<String> = branches.iter().map(|b| b.to_string()).collect();
    let branch_count = branch_slugs.len();
    ListLeafRow {
        slug: slug.to_string(),
        file: file.to_string(),
        display_title: display_title.to_string(),
        collected_at: collected_at.map(str::to_string),
        branches: branch_slugs,
        branch_count,
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
