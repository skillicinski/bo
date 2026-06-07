// bo list — deterministic tree inspection for collected leaves and compiled branches.

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

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ListOptions {
    pub view: ListViewMode,
    pub terms: Vec<String>,
    pub limit: Option<usize>,
    pub recent: bool,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub enum ListViewMode {
    #[default]
    BranchCentric,
    Branches,
    Leaves,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "mode")]
pub enum ListView {
    /// Default: branch-centric tree with leaves nested under each branch.
    /// Unbranched leaves at the end under an "unbranched" key.
    BranchCentric {
        branches: Vec<BranchWithLeaves>,
        unbranched: Vec<ListLeafRow>,
    },
    /// `--branches`: flat branch list.
    Branches { items: Vec<BranchRow> },
    /// `--leaves`: flat leaf list.
    Leaves { items: Vec<ListLeafRow> },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BranchWithLeaves {
    pub slug: String,
    pub title: String,
    pub updated_at: Option<String>,
    pub leaves: Vec<ListLeafRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BranchRow {
    pub slug: String,
    pub title: String,
    pub leaf_count: usize,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListLeafRow {
    pub slug: String,
    pub file: String,
    pub display_title: String,
    pub collected_at: Option<String>,
    pub branches: Vec<String>,
    pub branch_count: usize,
    pub degraded: bool,
    pub degradation_reasons: Vec<String>,

    #[serde(skip)]
    pub index_position: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ListResult {
    pub view: ListView,
    pub total_branches: usize,
    pub total_leaves: usize,
    pub branch_filter: Option<String>,
}

impl ListResult {
    /// Return an iterator over all degraded leaves across any view mode.
    pub fn degraded_leaves(&self) -> Vec<&ListLeafRow> {
        match &self.view {
            ListView::BranchCentric {
                branches,
                unbranched,
            } => {
                let mut rows: Vec<&ListLeafRow> = branches
                    .iter()
                    .flat_map(|b| b.leaves.iter())
                    .chain(unbranched.iter())
                    .filter(|r| r.degraded)
                    .collect();
                rows.sort_by_key(|r| r.index_position);
                rows
            }
            ListView::Branches { .. } => Vec::new(),
            ListView::Leaves { items: leaves } => leaves.iter().filter(|r| r.degraded).collect(),
        }
    }
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

// ── list_tree ────────────────────────────────────────────────────────────────

pub fn list_tree(tree_dir: &Path, options: &ListOptions) -> Result<ListResult, ListError> {
    let tree = Tree {
        name: "unnamed".to_string(),
        created_at: None,
        path: tree_dir.to_path_buf(),
    };
    let m = match manifest::read(&tree.manifest_path()) {
        Ok(m) => m,
        Err(manifest::ManifestError::TreeNotInitialized) => {
            return Ok(ListResult {
                view: ListView::BranchCentric {
                    branches: Vec::new(),
                    unbranched: Vec::new(),
                },
                total_branches: 0,
                total_leaves: 0,
                branch_filter: options.branch.clone(),
            });
        }
        Err(e) => return Err(ListError::Manifest(e)),
    };
    let total_branches = m.branches.len();
    let total_leaves = m.leaves.len();
    let canonical_tree_dir = fs::canonicalize(tree_dir).ok();

    // Build all leaf rows first (used by every view mode)
    let leaf_rows: Vec<ListLeafRow> = m
        .leaves
        .iter()
        .enumerate()
        .map(|(i, leaf)| build_row(tree_dir, canonical_tree_dir.as_deref(), leaf, &m, i))
        .collect();

    let view = match options.view {
        ListViewMode::BranchCentric => build_branch_centric(&m, &leaf_rows, options),
        ListViewMode::Branches => build_branches_view(&m, options),
        ListViewMode::Leaves => build_leaves_view(leaf_rows, options),
    };

    Ok(ListResult {
        view,
        total_branches,
        total_leaves,
        branch_filter: options.branch.clone(),
    })
}

fn build_branch_centric(
    manifest: &Manifest,
    leaf_rows: &[ListLeafRow],
    options: &ListOptions,
) -> ListView {
    // Collect slugs of leaves that belong to at least one branch
    let leaf_slug_to_row: std::collections::HashMap<&str, &ListLeafRow> =
        leaf_rows.iter().map(|r| (r.slug.as_str(), r)).collect();

    let mut branch_slugs_with_leaves: Vec<&str> = Vec::new();

    let mut branches: Vec<BranchWithLeaves> = manifest
        .branches
        .iter()
        .map(|b| {
            let leaves: Vec<ListLeafRow> = b
                .leaves
                .iter()
                .filter_map(|s| {
                    let row = leaf_slug_to_row.get(s.as_str()).copied()?;
                    if terms_match_row(row, &options.terms)
                        || terms_match_slug_title(b.slug.as_str(), b.title.as_str(), &options.terms)
                    {
                        Some(row.clone())
                    } else {
                        None
                    }
                })
                .collect();

            // Track which branches have associated leaf slugs
            if !b.leaves.is_empty() {
                branch_slugs_with_leaves.push(b.slug.as_str());
            }

            BranchWithLeaves {
                slug: b.slug.to_string(),
                title: b.title.to_string(),
                updated_at: Some(b.updated_at.to_rfc3339_millis()),
                leaves,
            }
        })
        .collect();

    // Apply --branch filter to branches
    if let Some(filter) = &options.branch {
        branches.retain(|b| b.slug == *filter);
    }

    // Apply --terms filter: keep branches that either match themselves or have matching leaves
    if !options.terms.is_empty() {
        branches.retain(|b| {
            terms_match_slug_title(&b.slug, &b.title, &options.terms) || !b.leaves.is_empty()
        });
    }

    // Collect unbranched leaves (not referenced by any branch)
    let mut unbranched: Vec<ListLeafRow> = leaf_rows
        .iter()
        .filter(|r| {
            let in_any_branch = branch_slugs_with_leaves.iter().any(|bs| {
                manifest
                    .branch_by_slug_str(bs)
                    .map(|b| b.leaves.iter().any(|s| s.as_str() == r.slug))
                    .unwrap_or(false)
            });
            !in_any_branch
        })
        .cloned()
        .collect();

    // Filter unbranched by terms and --branch
    if !options.terms.is_empty() {
        unbranched.retain(|r| terms_match_row(r, &options.terms));
    }
    if let Some(filter) = &options.branch {
        unbranched.retain(|r| r.branches.iter().any(|c| c == filter));
    }

    // --limit: cap branches in branch-centric view (analysis: option A)
    if let Some(limit) = options.limit {
        branches.truncate(limit);
    }

    ListView::BranchCentric {
        branches,
        unbranched,
    }
}

fn build_branches_view(manifest: &Manifest, options: &ListOptions) -> ListView {
    let mut rows: Vec<BranchRow> = manifest
        .branches
        .iter()
        .map(|b| BranchRow {
            slug: b.slug.to_string(),
            title: b.title.to_string(),
            leaf_count: b.leaves.len(),
            updated_at: Some(b.updated_at.to_rfc3339_millis()),
        })
        .collect();

    // --terms filter on branch title + slug
    if !options.terms.is_empty() {
        rows.retain(|r| terms_match_slug_title(&r.slug, &r.title, &options.terms));
    }

    // --branch filter
    if let Some(filter) = &options.branch {
        rows.retain(|r| &r.slug == filter);
    }

    // --limit: cap branch count
    if let Some(limit) = options.limit {
        rows.truncate(limit);
    }

    ListView::Branches { items: rows }
}

fn build_leaves_view(mut leaf_rows: Vec<ListLeafRow>, options: &ListOptions) -> ListView {
    // --branch filter
    if let Some(branch) = &options.branch {
        leaf_rows.retain(|row| row.branches.iter().any(|c| c == branch));
    }

    // --terms filter
    if !options.terms.is_empty() {
        leaf_rows.retain(|r| terms_match_row(r, &options.terms));
    }

    // --recent sort (only active in Leaves mode, per analysis)
    if options.recent {
        sort_rows_recent(&mut leaf_rows);
    }

    // --limit: cap leaf count
    if let Some(limit) = options.limit {
        leaf_rows.truncate(limit);
    }

    ListView::Leaves { items: leaf_rows }
}

// ── terms filtering ──────────────────────────────────────────────────────────

/// Check if a leaf row matches all given terms against its title and slug.
fn terms_match_row(row: &ListLeafRow, terms: &[String]) -> bool {
    terms_match_slug_title(&row.slug, &row.display_title, terms)
}

/// Check if slug + title match all terms (case-insensitive).
fn terms_match_slug_title(slug: &str, title: &str, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let slug_lower = slug.to_lowercase();
    let title_lower = title.to_lowercase();
    terms
        .iter()
        .all(|term| slug_lower.contains(term) || title_lower.contains(term))
}

// ── row builders ─────────────────────────────────────────────────────────────

fn build_row(
    tree_dir: &Path,
    canonical_tree_dir: Option<&Path>,
    leaf: &LeafRecord,
    manifest: &Manifest,
    index_position: usize,
) -> ListLeafRow {
    let display_title = if leaf.title.as_str().trim().is_empty() {
        filename_fallback(&leaf.file)
    } else {
        leaf.title.as_str().to_string()
    };
    let collected_at = if leaf.collected_at.to_rfc3339_millis().trim().is_empty() {
        None
    } else {
        Some(leaf.collected_at.to_rfc3339_millis())
    };
    let branch_slugs: Vec<String> = manifest
        .branches_for_leaf(&leaf.slug)
        .iter()
        .map(|b| b.slug.as_str().to_string())
        .collect();
    let branch_count = branch_slugs.len();

    let mut row = ListLeafRow {
        slug: leaf.slug.to_string(),
        file: leaf.file.clone(),
        display_title,
        collected_at,
        branches: branch_slugs,
        branch_count,
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
    if result.total_branches == 0 && result.total_leaves == 0 {
        return "no content in tree\n".to_string();
    }

    match &result.view {
        ListView::BranchCentric {
            branches,
            unbranched,
        } => {
            if branches.is_empty() && unbranched.is_empty() {
                return match &result.branch_filter {
                    Some(b) => format!("no branches matched '{b}'\n"),
                    None => {
                        if result.total_branches == 0 {
                            "no branches compiled yet\n".to_string()
                        } else {
                            "no leaves matched\n".to_string()
                        }
                    }
                };
            }

            let mut output = String::new();
            for branch in branches {
                output.push_str(&format!("## {}\n", branch.title));
                for leaf in &branch.leaves {
                    let collected_at = leaf.collected_at.as_deref().unwrap_or("-");
                    let branch_list = format!("[{}]", leaf.branches.join(", "));
                    output.push_str(&format!(
                        "  {} | {} | {}",
                        leaf.display_title, collected_at, branch_list
                    ));
                    if leaf.degraded {
                        output.push_str(&format!(
                            " | ⚠ DEGRADED: {}",
                            leaf.degradation_reasons.join(", ")
                        ));
                    }
                    output.push('\n');
                }
                output.push('\n');
            }

            if !unbranched.is_empty() {
                output.push_str("## unbranched\n");
                for leaf in unbranched {
                    let collected_at = leaf.collected_at.as_deref().unwrap_or("-");
                    let branch_list = format!("[{}]", leaf.branches.join(", "));
                    output.push_str(&format!(
                        "  {} | {} | {}",
                        leaf.display_title, collected_at, branch_list
                    ));
                    if leaf.degraded {
                        output.push_str(&format!(
                            " | ⚠ DEGRADED: {}",
                            leaf.degradation_reasons.join(", ")
                        ));
                    }
                    output.push('\n');
                }
            }

            output
        }
        ListView::Branches { items: rows } => {
            if rows.is_empty() {
                if result.total_branches == 0 {
                    return "no branches compiled yet\n".to_string();
                }
                return "no branches matched\n".to_string();
            }

            let mut output = String::new();
            for row in rows {
                output.push_str(&format!(
                    "{} | {} | {} leaves\n",
                    row.slug, row.title, row.leaf_count
                ));
            }
            output
        }
        ListView::Leaves { items: rows } => {
            if rows.is_empty() {
                if result.total_leaves == 0 {
                    return "no leaves collected yet\n".to_string();
                }
                if let Some(branch) = &result.branch_filter {
                    return format!("no leaves matched branch '{branch}'\n");
                }
                return "no leaves matched\n".to_string();
            }

            let mut output = String::new();
            for row in rows {
                let collected_at = row.collected_at.as_deref().unwrap_or("-");

                output.push_str(&format!(
                    "{} | {} | {} | {} branches",
                    row.display_title, row.slug, collected_at, row.branch_count
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
    }
}

#[cfg(test)]
#[path = "../tests/cli_list_tests.rs"]
mod tests;
