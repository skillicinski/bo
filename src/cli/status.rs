// bo status — tree health and compile readiness at a glance.
//
// Pipeline: read manifest → derive metrics → scan filesystem for size and
//           orphan/missing checks → return StatusResult.
//
// Read-only: never modifies any file. Reads consult the manifest.

use crate::domain::frontmatter;
use crate::domain::manifest;
use crate::domain::tree::{self, TreeRuntimeState};
use crate::domain::Timestamp;
use crate::engine::config::Config;
use crate::engine::llm::models;

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
    // Config fields
    pub provider: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compile_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_context_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compile_model_context_tokens: Option<usize>,
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
    Manifest(manifest::ManifestError),
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatusError::Io(msg) => write!(f, "{}", msg),
            StatusError::Manifest(e) => write!(f, "{}", e),
        }
    }
}

impl From<manifest::ManifestError> for StatusError {
    fn from(e: manifest::ManifestError) -> Self {
        StatusError::Manifest(e)
    }
}

// ── pipeline ──────────────────────────────────────────────────────────────────

pub fn compute_status(
    tree_dir: &Path,
    tree_name: &str,
    config: Option<&Config>,
) -> Result<StatusResult, StatusError> {
    let branches_dir = tree::branches_dir(tree_dir);

    let manifest = match tree::runtime_state(tree_dir) {
        Ok(TreeRuntimeState::Initialized(manifest)) => manifest,
        Ok(TreeRuntimeState::FreshSeeded) => manifest::Manifest {
            tree: manifest::TreeMeta {
                name: tree_name.to_string(),
                created_at: Timestamp::now(),
                last_compiled_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        },
        Ok(TreeRuntimeState::MissingManifest) => {
            return Err(StatusError::Manifest(
                manifest::ManifestError::TreeNotInitialized,
            ));
        }
        Err(error) => return Err(StatusError::Manifest(error)),
    };

    // Leaf metrics straight from the manifest.
    let uncompiled_slugs: Vec<String> = manifest
        .uncompiled_leaves()
        .iter()
        .map(|l| l.slug.as_str().to_string())
        .collect();

    let leaves = LeafStatus {
        total: manifest.leaves.len(),
        uncompiled: uncompiled_slugs.len(),
        uncompiled_slugs,
    };

    let branches = BranchStatus {
        total: manifest.branches.len(),
        last_compiled_at: manifest
            .tree
            .last_compiled_at
            .as_ref()
            .map(|t| t.to_rfc3339_millis()),
    };

    // Filesystem scan only for size — the one metric the manifest doesn't track.
    let size = compute_size(tree_dir, &branches_dir);

    let health = compute_health(tree_dir, &manifest);
    let hints = generate_hints(&leaves, &branches, &health);

    let (provider, model, compile_model, model_ctx, compile_ctx) = config_fields(config);

    Ok(StatusResult {
        tree_name: tree_name.to_string(),
        leaves,
        branches,
        size,
        health,
        hints,
        provider,
        model,
        compile_model,
        model_context_tokens: model_ctx,
        compile_model_context_tokens: compile_ctx,
    })
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn config_fields(
    config: Option<&Config>,
) -> (
    String,
    Option<String>,
    Option<String>,
    Option<usize>,
    Option<usize>,
) {
    let cfg = match config {
        Some(c) => c,
        None => return (String::from("openai"), None, None, None, None),
    };
    let provider = cfg.provider.to_string();
    let model_ctx = models::context_window_tokens(cfg.provider, &cfg.model);
    let compile_ctx = cfg
        .compile_model
        .as_deref()
        .and_then(|m| models::context_window_tokens(cfg.provider, m));
    (
        provider,
        Some(cfg.model.clone()),
        cfg.compile_model.clone(),
        model_ctx,
        compile_ctx,
    )
}

/// Return a StatusResult when the tree hasn't been seeded yet — config fields only.
pub fn config_only_status(config: Option<&Config>) -> StatusResult {
    let (provider, model, compile_model, model_ctx, compile_ctx) = config_fields(config);
    StatusResult {
        tree_name: String::new(),
        leaves: LeafStatus {
            total: 0,
            uncompiled: 0,
            uncompiled_slugs: Vec::new(),
        },
        branches: BranchStatus {
            total: 0,
            last_compiled_at: None,
        },
        size: SizeStatus {
            bytes: 0,
            estimated_tokens: 0,
        },
        health: HealthReport {
            orphan_index_entries: Vec::new(),
            missing_from_index: Vec::new(),
        },
        hints: vec!["run 'bo seed --path <path>' to create a tree".to_string()],
        provider,
        model,
        compile_model,
        model_context_tokens: model_ctx,
        compile_model_context_tokens: compile_ctx,
    }
}

fn compute_size(tree_dir: &Path, branches_dir: &Path) -> SizeStatus {
    let mut total_bytes: u64 = 0;

    for dir in [tree_dir, branches_dir] {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("md") {
                    if let Ok(meta) = fs::metadata(&path) {
                        total_bytes += meta.len();
                    }
                }
            }
        }
    }

    SizeStatus {
        bytes: total_bytes,
        estimated_tokens: total_bytes / 4,
    }
}

fn compute_health(tree_dir: &Path, manifest: &manifest::Manifest) -> HealthReport {
    // Orphan: manifest entry references a file that doesn't exist on disk.
    let orphans: Vec<OrphanEntry> = manifest
        .leaves
        .iter()
        .filter(|l| !tree_dir.join(&l.file).exists())
        .map(|l| OrphanEntry {
            file: l.file.clone(),
            title: l.title.as_str().to_string(),
            url: l.url.as_str().to_string(),
            remediation: format!("re-collect '{}' or remove the manifest entry", l.url),
        })
        .collect();

    let manifest_files: HashSet<&str> = manifest.leaves.iter().map(|l| l.file.as_str()).collect();
    let missing_from_index = scan_missing_from_index(tree_dir, &manifest_files);

    HealthReport {
        orphan_index_entries: orphans,
        missing_from_index,
    }
}

fn scan_missing_from_index(tree_dir: &Path, manifest_files: &HashSet<&str>) -> Vec<String> {
    let mut missing: Vec<String> = Vec::new();
    if let Ok(dir_entries) = fs::read_dir(tree_dir) {
        for entry in dir_entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().into_owned();
            if manifest_files.contains(filename.as_str()) {
                continue;
            }
            if is_leaf_file(&path) {
                missing.push(filename);
            }
        }
    }
    missing
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

    if !result.tree_name.is_empty() {
        out.push_str(&format!("bo \u{00b7} {}\n", result.tree_name));
    }
    out.push('\n');

    // Config display
    out.push_str(&format!("  provider:      {}\n", result.provider));
    if let Some(ref model) = result.model {
        let ctx = result
            .model_context_tokens
            .map(|t| format!(" ({}K context)", t / 1000))
            .unwrap_or_default();
        out.push_str(&format!("  model:         {}{}\n", model, ctx));
    }
    if let Some(ref cm) = result.compile_model {
        let ctx = result
            .compile_model_context_tokens
            .map(|t| format!(" ({}K context)", t / 1000))
            .unwrap_or_default();
        out.push_str(&format!("  compile_model: {}{}\n", cm, ctx));
    }

    out.push('\n');

    if result.leaves.uncompiled > 0 {
        out.push_str(&format!(
            "  Leaves:      {} ({} uncompiled)\n",
            result.leaves.total, result.leaves.uncompiled
        ));
    } else {
        out.push_str(&format!("  Leaves:      {}\n", result.leaves.total));
    }

    out.push_str(&format!("  Branches:    {}\n", result.branches.total));

    if let Some(ref ts) = result.branches.last_compiled_at {
        out.push_str(&format!("  Last compile: {}\n", ts));
    }

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
