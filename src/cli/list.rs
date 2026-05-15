// bo list — deterministic tree inspection for collected leaves.

use crate::domain::{frontmatter, index, tree};
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
    let index_path = tree::index_path(tree_dir);
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
#[path = "../tests/cli_list_tests.rs"]
mod tests;
