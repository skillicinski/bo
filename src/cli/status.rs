// bo status — tree health and compile readiness at a glance.
//
// Pipeline: read index → read state → scan filesystem → compute health →
//           derive hints → return StatusResult.
//
// Read-only: never modifies any file.

use crate::domain::{branch, frontmatter, index, tree};
use crate::engine::state;

use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;

// ── public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct StatusResult {
    pub tree_name: String,
    pub leaves: LeafStatus,
    pub branches: BranchStatus,
    pub size: SizeStatus,
    pub health: HealthReport,
    pub hints: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LeafStatus {
    pub total: usize,
    pub uncompiled: usize,
    pub uncompiled_slugs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchStatus {
    pub total: usize,
    pub last_compiled_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SizeStatus {
    pub bytes: u64,
    pub estimated_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub orphan_index_entries: Vec<OrphanEntry>,
    pub missing_from_index: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrphanEntry {
    pub file: String,
    pub title: String,
    pub url: String,
    pub remediation: String,
}

#[derive(Debug)]
pub enum StatusError {
    Io(String),
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatusError::Io(msg) => write!(f, "{}", msg),
        }
    }
}

// ── pipeline ──────────────────────────────────────────────────────────────────

pub fn compute_status(tree_dir: &Path, tree_name: &str) -> Result<StatusResult, StatusError> {
    let index_path = tree::index_path(tree_dir);
    let state_path = tree::state_path(tree_dir);
    let branches_dir = tree_dir.join("branches");

    // Read index
    let entries = index::read_index(&index_path)
        .map_err(|e| StatusError::Io(format!("failed to read index: {}", e)))?;

    // Read state
    let tree_state = state::read_state(&state_path);

    // Compute leaf stats
    let leaf_slugs: Vec<String> = entries
        .iter()
        .map(|e| state::slug_from_filename(&e.file).to_string())
        .collect();

    let uncompiled_slugs: Vec<String> = leaf_slugs
        .iter()
        .filter(|slug| !tree_state.compiled_leaves.contains_key(*slug))
        .cloned()
        .collect();

    let leaves = LeafStatus {
        total: entries.len(),
        uncompiled: uncompiled_slugs.len(),
        uncompiled_slugs,
    };

    // Scan branches
    let (branch_count, last_compiled_at) = scan_branches(&branches_dir);

    let branches = BranchStatus {
        total: branch_count,
        last_compiled_at,
    };

    // Compute size
    let size = compute_size(tree_dir, &branches_dir);

    // Health checks
    let health = compute_health(tree_dir, &entries);

    // Generate hints
    let hints = generate_hints(&leaves, &branches, &health);

    Ok(StatusResult {
        tree_name: tree_name.to_string(),
        leaves,
        branches,
        size,
        health,
        hints,
    })
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn scan_branches(branches_dir: &Path) -> (usize, Option<String>) {
    let mut count = 0usize;
    let mut latest: Option<String> = None;

    let entries = match fs::read_dir(branches_dir) {
        Ok(e) => e,
        Err(_) => return (0, None), // branches/ doesn't exist yet
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        count += 1;
        if let Some(compiled_at) = branch::read_compiled_at(&path) {
            match &latest {
                None => latest = Some(compiled_at),
                Some(existing) if compiled_at > *existing => latest = Some(compiled_at),
                _ => {}
            }
        }
    }

    (count, latest)
}

fn compute_size(tree_dir: &Path, branches_dir: &Path) -> SizeStatus {
    let mut total_bytes: u64 = 0;

    // Sum leaf files at tree root
    if let Ok(entries) = fs::read_dir(tree_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(meta) = fs::metadata(&path) {
                    total_bytes += meta.len();
                }
            }
        }
    }

    // Sum branch files
    if let Ok(entries) = fs::read_dir(branches_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
                if let Ok(meta) = fs::metadata(&path) {
                    total_bytes += meta.len();
                }
            }
        }
    }

    SizeStatus {
        bytes: total_bytes,
        estimated_tokens: total_bytes / 4,
    }
}

fn compute_health(tree_dir: &Path, entries: &[index::IndexEntry]) -> HealthReport {
    // Orphan detection: index entry references a file that doesn't exist
    let orphans: Vec<OrphanEntry> = entries
        .iter()
        .filter(|e| !tree_dir.join(&e.file).exists())
        .map(|e| OrphanEntry {
            file: e.file.clone(),
            title: e.title.clone(),
            url: e.url.clone(),
            remediation: format!("re-collect '{}' or remove the index entry", e.url),
        })
        .collect();

    // Missing detection: .md files on disk not in index
    let indexed_files: HashSet<&str> = entries.iter().map(|e| e.file.as_str()).collect();

    let mut missing: Vec<String> = Vec::new();
    if let Ok(dir_entries) = fs::read_dir(tree_dir) {
        for entry in dir_entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().into_owned();
            if indexed_files.contains(filename.as_str()) {
                continue;
            }
            // Only flag as missing if it looks like a leaf (has url: in frontmatter)
            if is_leaf_file(&path) {
                missing.push(filename);
            }
        }
    }

    HealthReport {
        orphan_index_entries: orphans,
        missing_from_index: missing,
    }
}

/// Check if a .md file is a leaf by looking for `url:` in its frontmatter.
fn is_leaf_file(path: &Path) -> bool {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return false,
    };
    match frontmatter::parse(&content) {
        Ok((mapping, _)) => mapping.get("url").is_some(),
        Err(_) => false,
    }
}

fn generate_hints(
    leaves: &LeafStatus,
    branches: &BranchStatus,
    health: &HealthReport,
) -> Vec<String> {
    let mut hints = Vec::new();

    if leaves.total == 0 {
        hints.push("run 'bo collect <url>' to add your first source".to_string());
    } else if leaves.uncompiled > 0 && branches.total == 0 {
        hints.push(format!(
            "run 'bo compile' to create your first branch from {} leaves",
            leaves.total
        ));
    } else if leaves.uncompiled > 0 {
        hints.push(format!(
            "run 'bo compile' to process {} new leaves",
            leaves.uncompiled
        ));
    }

    if !health.orphan_index_entries.is_empty() {
        let n = health.orphan_index_entries.len();
        hints.push(format!(
            "{} index {} reference missing files \u{2014} re-collect or remove manually",
            n,
            if n == 1 { "entry" } else { "entries" }
        ));
    }

    if !health.missing_from_index.is_empty() {
        let n = health.missing_from_index.len();
        hints.push(format!(
            "{} leaf {} not indexed \u{2014} they won't appear in search or compile",
            n,
            if n == 1 { "file" } else { "files" }
        ));
    }

    hints
}

// ── output formatting ─────────────────────────────────────────────────────────

const UNCOMPILED_DISPLAY_CAP: usize = 10;

pub fn render_human(result: &StatusResult) -> String {
    let mut out = String::new();

    out.push_str(&format!("bo \u{00b7} {}\n", result.tree_name));
    out.push('\n');

    // Leaves
    if result.leaves.uncompiled > 0 {
        out.push_str(&format!(
            "  Leaves:      {} ({} uncompiled)\n",
            result.leaves.total, result.leaves.uncompiled
        ));
    } else {
        out.push_str(&format!("  Leaves:      {}\n", result.leaves.total));
    }

    // Branches
    out.push_str(&format!("  Branches:    {}\n", result.branches.total));

    // Last compile
    if let Some(ref ts) = result.branches.last_compiled_at {
        out.push_str(&format!("  Last compile: {}\n", ts));
    }

    // Size
    let kb = result.size.bytes / 1024;
    let display_size = if kb > 0 {
        format!("{} KB", kb)
    } else {
        format!("{} B", result.size.bytes)
    };
    out.push_str(&format!(
        "  Size:        {} (~{} tokens)\n",
        display_size,
        format_number(result.size.estimated_tokens)
    ));

    // Uncompiled list
    if !result.leaves.uncompiled_slugs.is_empty() {
        out.push('\n');
        out.push_str("  Uncompiled:\n");
        let display_count = result
            .leaves
            .uncompiled_slugs
            .len()
            .min(UNCOMPILED_DISPLAY_CAP);
        for slug in &result.leaves.uncompiled_slugs[..display_count] {
            out.push_str(&format!("    \u{2022} {}\n", slug));
        }
        let remaining = result.leaves.uncompiled_slugs.len() - display_count;
        if remaining > 0 {
            out.push_str(&format!("    \u{2026} and {} more\n", remaining));
        }
    }

    // Health issues
    if !result.health.orphan_index_entries.is_empty()
        || !result.health.missing_from_index.is_empty()
    {
        out.push('\n');
        out.push_str("  Issues:\n");
        for orphan in &result.health.orphan_index_entries {
            out.push_str(&format!(
                "    \u{26a0} orphan: {} ({})\n",
                orphan.file, orphan.remediation
            ));
        }
        for missing in &result.health.missing_from_index {
            out.push_str(&format!(
                "    \u{26a0} not indexed: {} (run 'bo collect' to re-index)\n",
                missing
            ));
        }
    }

    // Hints
    if !result.hints.is_empty() {
        out.push('\n');
        for hint in &result.hints {
            out.push_str(&format!("  \u{2192} {}\n", hint));
        }
    }

    out
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
#[path = "../tests/cli_status_tests.rs"]
mod tests;
