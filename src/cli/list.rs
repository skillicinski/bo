// bo list — deterministic tree inspection for collected leaves.

use crate::cli::json::JsonError;
use crate::domain::manifest::{self, LeafRecord, Manifest};
use crate::domain::tree::Tree;
use chrono::{DateTime, FixedOffset};
use serde::Serialize;
use std::cmp::Ordering;
use std::fmt;
use std::fs;
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
    Manifest(manifest::ManifestError),
}

impl fmt::Display for ListError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ListError::Io(e) => write!(f, "I/O error: {}", e),
            ListError::Json(e) => write!(f, "JSON error: {}", e),
            ListError::Manifest(e) => write!(f, "{}", e),
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

impl From<manifest::ManifestError> for ListError {
    fn from(e: manifest::ManifestError) -> Self {
        ListError::Manifest(e)
    }
}

impl ListError {
    pub fn code(&self) -> &'static str {
        match self {
            ListError::Io(_) => "io_error",
            ListError::Json(_) => "json_error",
            ListError::Manifest(_) => "manifest_error",
        }
    }

    pub fn json_error(&self) -> JsonError {
        JsonError::new(self.code(), self.to_string())
    }
}

// ── list ─────────────────────────────────────────────────────────────────────

pub fn list_leaves(tree_dir: &Path, options: &ListOptions) -> Result<ListResult, ListError> {
    let tree = Tree {
        name: None,
        created_at: None,
        output_dir: tree_dir.to_path_buf(),
    };
    let m = match manifest::read(&tree.manifest_path()) {
        Ok(m) => m,
        Err(manifest::ManifestError::TreeNotInitialized) => {
            return Ok(ListResult {
                leaves: Vec::new(),
                total_index_entries: 0,
                branch_filter: options.branch.clone(),
            });
        }
        Err(e) => return Err(ListError::Manifest(e)),
    };
    let total_index_entries = m.leaves.len();
    let canonical_tree_dir = fs::canonicalize(tree_dir).ok();

    let mut leaves: Vec<ListLeafRow> = m
        .leaves
        .iter()
        .enumerate()
        .map(|(i, leaf)| build_row(tree_dir, canonical_tree_dir.as_deref(), leaf, &m, i))
        .collect();

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
    leaf: &LeafRecord,
    manifest: &Manifest,
    index_position: usize,
) -> ListLeafRow {
    let display_title = if leaf.title.trim().is_empty() {
        filename_fallback(&leaf.file)
    } else {
        leaf.title.clone()
    };
    let collected_at = if leaf.collected_at.trim().is_empty() {
        None
    } else {
        Some(leaf.collected_at.clone())
    };
    let branches: Vec<String> = manifest
        .branches_for_leaf(&leaf.slug)
        .iter()
        .map(|b| b.slug.clone())
        .collect();

    let mut row = ListLeafRow {
        file: leaf.file.clone(),
        display_title,
        collected_at,
        branches,
        degraded: false,
        degradation_reasons: Vec::new(),
        index_position,
    };

    // Path safety + file-existence checks remain the only degradation signals
    // post-manifest. Frontmatter-derived issues are obsolete; if invalid data
    // ever reached the manifest, it indicates a writer bug, not a runtime
    // condition list should soft-handle.
    let path = match resolve_leaf_path(tree_dir, canonical_tree_dir, &leaf.file) {
        Ok(path) => path,
        Err(reason) => {
            push_degradation_reason(&mut row, reason);
            return row;
        }
    };

    if !path.exists() {
        push_degradation_reason(&mut row, "missing file");
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
