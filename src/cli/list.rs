// bo list — deterministic tree inspection for collected leaves.

use crate::domain::{frontmatter, index};
use chrono::{DateTime, FixedOffset};
use serde::Serialize;
use serde_yaml_ng::{Mapping, Value};
use std::cmp::Ordering;
use std::fmt;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

// ── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct ListOptions {
    pub limit: Option<usize>,
    pub recent: bool,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct ListLeafRow {
    pub file: String,
    pub display_title: String,
    pub collected_at: Option<String>,
    pub branches: Vec<String>,
    pub degraded: bool,
    pub degradation_reasons: Vec<String>,

    #[serde(skip)]
    pub index_position: usize,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct ListResult {
    pub leaves: Vec<ListLeafRow>,
    pub total_index_entries: usize,
    pub branch_filter: Option<String>,
}

#[derive(Debug)]
pub enum ListError {
    Io(std::io::Error),
    Json(serde_json::Error),
}

impl fmt::Display for ListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListError::Io(e) => write!(f, "I/O error: {}", e),
            ListError::Json(e) => write!(f, "JSON error: {}", e),
        }
    }
}

impl From<std::io::Error> for ListError {
    fn from(e: std::io::Error) -> Self {
        ListError::Io(e)
    }
}

impl From<serde_json::Error> for ListError {
    fn from(e: serde_json::Error) -> Self {
        ListError::Json(e)
    }
}

// ── list ─────────────────────────────────────────────────────────────────────

pub fn list_leaves(tree_dir: &Path, options: &ListOptions) -> Result<ListResult, ListError> {
    let index_path = tree_dir.join("index.jsonl");
    let entries = index::read_index(&index_path)?;
    let total_index_entries = entries.len();
    let canonical_tree_dir = fs::canonicalize(tree_dir).ok();

    let mut leaves = entries
        .iter()
        .enumerate()
        .map(|(index_position, entry)| {
            build_row(
                tree_dir,
                canonical_tree_dir.as_deref(),
                entry,
                index_position,
            )
        })
        .collect::<Vec<_>>();

    if let Some(branch) = &options.branch {
        leaves.retain(|row| row.branches.iter().any(|candidate| candidate == branch));
    }

    if options.recent {
        sort_rows_recent(&mut leaves);
    }

    if let Some(limit) = options.limit {
        leaves.truncate(limit);
    }

    Ok(ListResult {
        leaves,
        total_index_entries,
        branch_filter: options.branch.clone(),
    })
}

fn build_row(
    tree_dir: &Path,
    canonical_tree_dir: Option<&Path>,
    entry: &index::IndexEntry,
    index_position: usize,
) -> ListLeafRow {
    let mut row = ListLeafRow {
        file: entry.file.clone(),
        display_title: fallback_display_title(None, &entry.title, &entry.file),
        collected_at: None,
        branches: Vec::new(),
        degraded: false,
        degradation_reasons: Vec::new(),
        index_position,
    };

    let path = match resolve_leaf_path(tree_dir, canonical_tree_dir, &entry.file) {
        Ok(path) => path,
        Err(reason) => {
            push_degradation_reason(&mut row, reason);
            return row;
        }
    };

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            push_degradation_reason(&mut row, "missing file");
            return row;
        }
        Err(_) => {
            push_degradation_reason(&mut row, "unreadable file");
            return row;
        }
    };

    let mapping = match frontmatter::parse(&content) {
        Ok((mapping, _)) => mapping,
        Err(_) => {
            push_degradation_reason(&mut row, "invalid frontmatter");
            return row;
        }
    };

    row.display_title =
        fallback_display_title(frontmatter_title(&mapping), &entry.title, &entry.file);

    match extract_collected_at(&mapping) {
        Ok(value) => row.collected_at = value,
        Err(reason) => push_degradation_reason(&mut row, reason),
    }

    let (branches, branch_reason) = extract_branches(&mapping);
    row.branches = branches;
    if let Some(reason) = branch_reason {
        push_degradation_reason(&mut row, reason);
    }

    row
}

fn resolve_leaf_path(
    tree_dir: &Path,
    canonical_tree_dir: Option<&Path>,
    file: &str,
) -> Result<PathBuf, &'static str> {
    let relative = Path::new(file);

    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || has_disallowed_components(relative)
    {
        return Err("suspicious path");
    }

    let resolved = tree_dir.join(relative);

    if let Some(canonical_root) = canonical_tree_dir {
        if resolved.exists() {
            let canonical_resolved = fs::canonicalize(&resolved).map_err(|_| "suspicious path")?;
            if !canonical_resolved.starts_with(canonical_root) {
                return Err("suspicious path");
            }
        } else if let Some(parent) = resolved.parent() {
            if parent.exists() {
                let canonical_parent = fs::canonicalize(parent).map_err(|_| "suspicious path")?;
                if !canonical_parent.starts_with(canonical_root) {
                    return Err("suspicious path");
                }
            }
        }
    }

    Ok(resolved)
}

#[cfg(windows)]
fn has_disallowed_components(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
}

#[cfg(not(windows))]
fn has_disallowed_components(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn frontmatter_title(mapping: &Mapping) -> Option<&str> {
    mapping
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
}

fn fallback_display_title(leaf_title: Option<&str>, index_title: &str, file: &str) -> String {
    leaf_title
        .filter(|title| !title.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            let trimmed = index_title.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        })
        .unwrap_or_else(|| filename_fallback(file))
}

fn filename_fallback(file: &str) -> String {
    let path = Path::new(file);

    path.file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .map(str::to_string)
        .or_else(|| {
            path.file_name()
                .and_then(|name| name.to_str())
                .filter(|name| !name.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| file.to_string())
}

fn extract_collected_at(mapping: &Mapping) -> Result<Option<String>, &'static str> {
    let value = mapping.get("collected_at").ok_or("missing collected_at")?;
    let raw = value.as_str().ok_or("invalid collected_at")?.trim();

    if raw.is_empty() {
        return Err("invalid collected_at");
    }

    DateTime::parse_from_rfc3339(raw)
        .map(|_| Some(raw.to_string()))
        .map_err(|_| "invalid collected_at")
}

fn extract_branches(mapping: &Mapping) -> (Vec<String>, Option<&'static str>) {
    let Some(value) = mapping.get("branches") else {
        return (Vec::new(), None);
    };

    let Some(sequence) = value.as_sequence() else {
        return (Vec::new(), Some("invalid branches"));
    };

    let mut branches = Vec::new();
    let mut invalid = false;

    for item in sequence {
        match item.as_str() {
            Some(branch) => branches.push(branch.to_string()),
            None => invalid = true,
        }
    }

    (branches, invalid.then_some("invalid branches"))
}

fn push_degradation_reason(row: &mut ListLeafRow, reason: &'static str) {
    if !row
        .degradation_reasons
        .iter()
        .any(|existing| existing == reason)
    {
        row.degradation_reasons.push(reason.to_string());
    }
    row.degraded = true;
}

fn sort_rows_recent(rows: &mut [ListLeafRow]) {
    rows.sort_by(
        |left, right| match (parsed_collected_at(left), parsed_collected_at(right)) {
            (Some(left_date), Some(right_date)) => right_date
                .cmp(&left_date)
                .then_with(|| left.index_position.cmp(&right.index_position)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => left.index_position.cmp(&right.index_position),
        },
    );
}

fn parsed_collected_at(row: &ListLeafRow) -> Option<DateTime<FixedOffset>> {
    row.collected_at
        .as_deref()
        .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
}

// ── render ───────────────────────────────────────────────────────────────────

pub fn render_human(result: &ListResult) -> String {
    if result.total_index_entries == 0 {
        return "no leaves collected yet\n".to_string();
    }

    if result.leaves.is_empty() {
        if let Some(branch) = &result.branch_filter {
            return format!("no leaves matched branch '{branch}'\n");
        }
        return "no leaves matched\n".to_string();
    }

    let mut output = String::new();
    for row in &result.leaves {
        let collected_at = row.collected_at.as_deref().unwrap_or("-");
        let branches = format!("[{}]", row.branches.join(", "));

        output.push_str(&format!(
            "{} | {} | {}",
            row.display_title, collected_at, branches
        ));

        if row.degraded {
            output.push_str(&format!(
                " | ⚠ DEGRADED: {}",
                row.degradation_reasons.join(", ")
            ));
        }

        output.push('\n');
    }

    output
}

pub fn render_json(result: &ListResult) -> Result<String, ListError> {
    serde_json::to_string_pretty(result).map_err(ListError::from)
}

#[cfg(test)]
mod tests {
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
}
