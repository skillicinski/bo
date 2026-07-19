// bo status — tree health and synthesis readiness at a glance.
//
// Pipeline: read state → derive metrics → scan filesystem for size and
//           orphan/missing checks → return StatusResult.
//
// Read-only: never modifies any file. Reads consult the state.

use crate::domain::frontmatter;
use crate::domain::state;
use crate::domain::tree::{self, TreeLoadState};
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
    pub synthesis_model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_context_tokens: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub synthesis_model_context_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LeafStatus {
    pub total: usize,
    pub unsynthesized: usize,
    pub unsynthesized_slugs: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchStatus {
    pub total: usize,
    pub last_synthesized_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SizeStatus {
    pub bytes: u64,
    pub estimated_tokens: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthReport {
    pub missing_leaf_files: Vec<MissingLeafFile>,
    pub untracked_leaf_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MissingLeafFile {
    pub file: String,
    pub title: String,
    pub url: String,
    pub remediation: String,
}

#[derive(Debug)]
pub enum StatusError {
    Io(String),
    TreeState(state::TreeStateError),
}

impl std::fmt::Display for StatusError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StatusError::Io(msg) => write!(f, "{}", msg),
            StatusError::TreeState(e) => write!(f, "{}", e),
        }
    }
}

impl From<state::TreeStateError> for StatusError {
    fn from(e: state::TreeStateError) -> Self {
        StatusError::TreeState(e)
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

pub fn run(config: Option<Config>) -> Result<StatusResult, StatusError> {
    let Some(tree) = config
        .clone()
        .and_then(Config::into_seeded)
        .map(|c| c.tree())
    else {
        return Ok(config_only_status(config.as_ref()));
    };

    compute_status(tree.path(), &tree.name, config.as_ref())
}

// ── pipeline ──────────────────────────────────────────────────────────────────

pub fn compute_status(
    tree_dir: &Path,
    tree_name: &str,
    config: Option<&Config>,
) -> Result<StatusResult, StatusError> {
    let branch_dir = tree::branch_dir(tree_dir);
    let leaf_dir = tree::leaf_dir(tree_dir);

    let state = match crate::engine::state::load_state(tree_dir) {
        Ok(TreeLoadState::Loaded(state)) => state,
        Ok(TreeLoadState::FreshSeeded) => state::TreeState {
            tree: state::TreeMetadata {
                name: tree_name.to_string(),
                created_at: Timestamp::now(),
                last_synthesized_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        },
        Ok(TreeLoadState::MissingState) => {
            return Err(StatusError::TreeState(
                state::TreeStateError::TreeNotInitialized,
            ));
        }
        Err(error) => return Err(StatusError::TreeState(error)),
    };

    // Leaf metrics straight from the state.
    let unsynthesized_slugs: Vec<String> = state
        .unsynthesized_leaves()
        .iter()
        .map(|l| l.slug.as_str().to_string())
        .collect();

    let leaves = LeafStatus {
        total: state.leaves.len(),
        unsynthesized: unsynthesized_slugs.len(),
        unsynthesized_slugs,
    };

    let branches = BranchStatus {
        total: state.branches.len(),
        last_synthesized_at: state
            .tree
            .last_synthesized_at
            .as_ref()
            .map(|t| t.to_rfc3339_millis()),
    };

    // Filesystem scan only for size — the one metric the state doesn't track.
    let size = compute_size(&leaf_dir, &branch_dir);

    let health = compute_health(tree_dir, &state);
    let hints = generate_hints(&leaves, &branches, &health);

    let (provider, model, synthesis_model, model_ctx, synthesis_ctx) = config_fields(config);

    Ok(StatusResult {
        tree_name: tree_name.to_string(),
        leaves,
        branches,
        size,
        health,
        hints,
        provider,
        model,
        synthesis_model,
        model_context_tokens: model_ctx,
        synthesis_model_context_tokens: synthesis_ctx,
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
    let synthesis_ctx = cfg
        .synthesis_model
        .as_deref()
        .and_then(|m| models::context_window_tokens(cfg.provider, m));
    (
        provider,
        Some(cfg.model.clone()),
        cfg.synthesis_model.clone(),
        model_ctx,
        synthesis_ctx,
    )
}

/// Return a StatusResult when the tree hasn't been seeded yet — config fields only.
fn config_only_status(config: Option<&Config>) -> StatusResult {
    let (provider, model, synthesis_model, model_ctx, synthesis_ctx) = config_fields(config);
    StatusResult {
        tree_name: String::new(),
        leaves: LeafStatus {
            total: 0,
            unsynthesized: 0,
            unsynthesized_slugs: Vec::new(),
        },
        branches: BranchStatus {
            total: 0,
            last_synthesized_at: None,
        },
        size: SizeStatus {
            bytes: 0,
            estimated_tokens: 0,
        },
        health: HealthReport {
            missing_leaf_files: Vec::new(),
            untracked_leaf_files: Vec::new(),
        },
        hints: vec!["run 'bo seed --path <path>' to create a tree".to_string()],
        provider,
        model,
        synthesis_model,
        model_context_tokens: model_ctx,
        synthesis_model_context_tokens: synthesis_ctx,
    }
}

fn compute_size(leaf_dir: &Path, branch_dir: &Path) -> SizeStatus {
    let mut total_bytes: u64 = 0;

    for dir in [leaf_dir, branch_dir] {
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

fn compute_health(tree_dir: &Path, state: &state::TreeState) -> HealthReport {
    // Orphan: state entry references a file that doesn't exist on disk.
    let orphans: Vec<MissingLeafFile> = state
        .leaves
        .iter()
        .filter(|l| !tree_dir.join(&l.file).exists())
        .map(|l| MissingLeafFile {
            file: l.file.clone(),
            title: l
                .title
                .as_ref()
                .map(|t| t.as_str().to_string())
                .unwrap_or_default(),
            url: l.url.as_str().to_string(),
            remediation: format!("re-collect '{}' or remove the state entry", l.url),
        })
        .collect();

    let state_files: HashSet<&str> = state.leaves.iter().map(|l| l.file.as_str()).collect();
    let untracked_leaf_files = scan_untracked_leaf_files(tree_dir, &state_files);

    HealthReport {
        missing_leaf_files: orphans,
        untracked_leaf_files,
    }
}

fn scan_untracked_leaf_files(tree_dir: &Path, state_files: &HashSet<&str>) -> Vec<String> {
    let mut missing: Vec<String> = Vec::new();
    let leaf_dir = tree_dir.join("leaf");
    if let Ok(dir_entries) = fs::read_dir(&leaf_dir) {
        for entry in dir_entries.flatten() {
            let path = entry.path();
            if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let filename = entry.file_name().to_string_lossy().into_owned();
            // TreeState stores leaf paths as `leaf/{file}`; the on-disk entry is bare `{file}`.
            let relative = format!("leaf/{filename}");
            if state_files.contains(relative.as_str()) {
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
    } else if leaves.unsynthesized > 0 && branches.total == 0 {
        hints.push(format!(
            "run 'bo synthesize' to create your first branch from {} leaves",
            leaves.total
        ));
    } else if leaves.unsynthesized > 0 {
        hints.push(format!(
            "run 'bo synthesize' to process {} new leaves",
            leaves.unsynthesized
        ));
    }

    if !health.missing_leaf_files.is_empty() {
        let n = health.missing_leaf_files.len();
        hints.push(format!(
            "{} state {} reference missing files \u{2014} re-collect or remove manually",
            n,
            if n == 1 { "entry" } else { "entries" }
        ));
    }

    if !health.untracked_leaf_files.is_empty() {
        let n = health.untracked_leaf_files.len();
        hints.push(format!(
            "{} leaf {} untracked \u{2014} they won't appear in search or synthesis",
            n,
            if n == 1 { "file" } else { "files" }
        ));
    }

    hints
}

// ── output formatting ─────────────────────────────────────────────────────────

const UNSYNTHESIZED_DISPLAY_CAP: usize = 10;

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
    if let Some(ref cm) = result.synthesis_model {
        let ctx = result
            .synthesis_model_context_tokens
            .map(|t| format!(" ({}K context)", t / 1000))
            .unwrap_or_default();
        out.push_str(&format!("  synthesis_model: {}{}\n", cm, ctx));
    }

    out.push('\n');

    if result.leaves.unsynthesized > 0 {
        out.push_str(&format!(
            "  Leaves:      {} ({} unsynthesized)\n",
            result.leaves.total, result.leaves.unsynthesized
        ));
    } else {
        out.push_str(&format!("  Leaves:      {}\n", result.leaves.total));
    }

    out.push_str(&format!("  Branches:    {}\n", result.branches.total));

    if let Some(ref ts) = result.branches.last_synthesized_at {
        out.push_str(&format!("  Last synthesis: {}\n", ts));
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

    if !result.leaves.unsynthesized_slugs.is_empty() {
        out.push('\n');
        out.push_str("  Unsynthesized:\n");
        let display_count = result
            .leaves
            .unsynthesized_slugs
            .len()
            .min(UNSYNTHESIZED_DISPLAY_CAP);
        for slug in &result.leaves.unsynthesized_slugs[..display_count] {
            out.push_str(&format!("    \u{2022} {}\n", slug));
        }
        let remaining = result.leaves.unsynthesized_slugs.len() - display_count;
        if remaining > 0 {
            out.push_str(&format!("    \u{2026} and {} more\n", remaining));
        }
    }

    if !result.health.missing_leaf_files.is_empty()
        || !result.health.untracked_leaf_files.is_empty()
    {
        out.push('\n');
        out.push_str("  Issues:\n");
        for orphan in &result.health.missing_leaf_files {
            out.push_str(&format!(
                "    \u{26a0} orphan: {} ({})\n",
                orphan.file, orphan.remediation
            ));
        }
        for missing in &result.health.untracked_leaf_files {
            out.push_str(&format!(
                "    \u{26a0} untracked: {} (run 'bo collect' to track it)\n",
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
