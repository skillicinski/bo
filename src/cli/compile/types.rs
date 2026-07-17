// ── compile types: options, outcomes, errors, constants ──────────────────────

use std::time::Duration;

use serde::Serialize;
use serde_json::json;

use crate::cli::json::{JsonError, JsonWarning};
use crate::engine::llm::{LlmCallPolicy, Model, Usage};
use crate::engine::pending;

// ── constants ─────────────────────────────────────────────────────────────────

pub(super) const MAX_COMPLETION_TOKENS: u32 = 16384;
pub(super) const MAX_COMPILED_BODY_BYTES_MIN: usize = 16 * 1024;
pub(super) const MAX_COMPILED_BODY_BYTES_PER_INPUT_BYTE: usize = 8;
pub(super) const COMPILE_PROMPT_OVERHEAD_TOKENS: usize = 4096;
pub(super) const TOKEN_ESTIMATE_BYTES_PER_TOKEN: usize = 4;
pub(super) const NO_NEW_LEAVES_REASON: &str = "no new leaves since last compile";
const COMPILE_MODEL_NEXT_STEPS: [&str; 2] = [
    "bo config --compile-model gpt-4.1-mini",
    "bo config --compile-model gpt-4.1",
];

pub const VALIDATION_NEXT_STEP: &str = "No files were changed. Try `bo compile` again; if this repeats, switch models with `bo config --model <model>` or report the validation message.";

pub(super) const COMPILE_LLM_POLICY: LlmCallPolicy = LlmCallPolicy {
    timeout: Duration::from_secs(180),
    max_attempts: 3,
    initial_backoff: Duration::from_secs(2),
};

/// Leaf count threshold above which Full mode switches to two-stage compile.
pub(super) const TWO_STAGE_FULL_THRESHOLD: usize = 40;

/// New-leaf count threshold above which Incremental mode switches to two-stage.
pub(super) const TWO_STAGE_INCREMENTAL_THRESHOLD: usize = 15;

// ── errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CompileError {
    /// Collection exceeds the model's context window.
    ContextOverflow {
        model: String,
        estimated_tokens: Option<usize>,
        context_tokens: Option<usize>,
    },
    /// LLM output was truncated (hit max_completion_tokens).
    Truncated,
    /// Response blocked by content filter.
    ContentFilter,
    /// LLM API or network error.
    Llm(String),
    /// I/O or manifest/pending error.
    Io(String),
    /// Another bo process is mutating this tree.
    Busy(String),
    /// Validation error in the LLM response.
    Validation(String),
    /// A dry-run was blocked because recovery/repair would be required, or the
    /// manifest changed mid-run. Zero bytes were written.
    DryRunBlocked(String),
    /// The agent loop failed to produce a valid plan (limit, truncation,
    /// context overflow, provider error, or no submission). Zero bytes written.
    /// Carries the same resource diagnostics as a success envelope so error
    /// JSON carries turns/tool_calls/usage/last_error/signals_sent.
    AgentFailed {
        message: String,
        turns: usize,
        tool_calls: usize,
        usage: Option<Usage>,
        last_error: Option<String>,
        signals_sent: usize,
    },
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::ContextOverflow { model, .. } => write!(
                f,
                "compile model context is too small for '{}' — set a larger compile model, for example:\n{}\n{}",
                model, COMPILE_MODEL_NEXT_STEPS[0], COMPILE_MODEL_NEXT_STEPS[1]
            ),
            CompileError::Truncated => write!(
                f,
                "compile output was truncated — try reducing collection size or \
                 using a model with larger output capacity"
            ),
            CompileError::ContentFilter => write!(f, "compile was blocked by content filter"),
            CompileError::Llm(msg) => write!(f, "LLM error: {}", msg),
            CompileError::Io(msg) => write!(f, "{}", msg),
            CompileError::Busy(msg) => write!(f, "{}", msg),
            CompileError::Validation(msg) => write!(f, "{}\n{}", msg, VALIDATION_NEXT_STEP),
            CompileError::DryRunBlocked(msg) => write!(f, "{}", msg),
            CompileError::AgentFailed { message, .. } => write!(f, "{}", message),
        }
    }
}

impl From<pending::PendingError> for CompileError {
    fn from(error: pending::PendingError) -> Self {
        match error {
            pending::PendingError::Busy { .. } => CompileError::Busy(error.to_string()),
            other => CompileError::Io(other.to_string()),
        }
    }
}

impl CompileError {
    pub fn json_error(&self) -> JsonError {
        match self {
            CompileError::ContextOverflow {
                model,
                estimated_tokens,
                context_tokens,
            } => JsonError::with_details(
                "context_overflow",
                self.to_string(),
                json!({
                    "model": model,
                    "estimated_tokens": estimated_tokens,
                    "context_tokens": context_tokens,
                    "next_steps": COMPILE_MODEL_NEXT_STEPS,
                }),
            ),
            CompileError::Truncated => JsonError::new("truncated", self.to_string()),
            CompileError::ContentFilter => JsonError::new("content_filter", self.to_string()),
            CompileError::Llm(_) => JsonError::new("llm_error", self.to_string()),
            CompileError::Io(_) => JsonError::new("io_error", self.to_string()),
            CompileError::Busy(_) => JsonError::new("tree_busy", self.to_string()),
            CompileError::Validation(message) => JsonError::with_details(
                "validation_error",
                message.clone(),
                json!({
                    "phase": "compile_validation",
                    "files_changed": false,
                    "next_step": VALIDATION_NEXT_STEP,
                }),
            ),
            CompileError::DryRunBlocked(message) => JsonError::with_details(
                "dry_run_blocked",
                message.clone(),
                json!({ "files_changed": false }),
            ),
            CompileError::AgentFailed {
                message,
                turns,
                tool_calls,
                usage,
                last_error,
                signals_sent,
            } => JsonError::with_details(
                "agent_error",
                message.clone(),
                json!({
                    "files_changed": false,
                    "turns": turns,
                    "tool_calls": tool_calls,
                    "signals_sent": signals_sent,
                    "usage": usage,
                    "last_error": last_error,
                }),
            ),
        }
    }
}

// ── public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompileOptions {
    pub all: bool,
    /// Use the iterative agent path. Requires `dry_run` in this milestone.
    pub agent: bool,
    /// Produce a read-only validated preview; write zero bytes to the tree.
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompileRunMode {
    Incremental,
    Full,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompileResult {
    pub status: String,
    pub reason: Option<String>,
    pub mode: Option<CompileRunMode>,
    pub model: Option<String>,
    pub branches: Vec<BranchResult>,
    pub leaves_processed: usize,
    pub leaves_skipped: Vec<String>,
    #[serde(skip)]
    pub notifications: Vec<String>,
    /// Stderr-bound lines (title-collision warnings, pending-recovery notices,
    /// per-branch write progress). Skipped from JSON — these are presentation,
    /// never part of the data envelope. Populated by the entry point.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl CompileResult {
    pub fn compiled(
        summary: CompileSummary,
        mode: CompileRunMode,
        model: &Model,
        notifications: Vec<String>,
    ) -> Self {
        Self {
            status: "compiled".to_string(),
            reason: None,
            mode: Some(mode),
            model: Some(model.to_string()),
            branches: summary.branches,
            leaves_processed: summary.leaves_processed,
            leaves_skipped: summary.leaves_skipped,
            notifications,
            warnings: Vec::new(),
        }
    }

    pub fn noop(reason: &str, notifications: Vec<String>) -> Self {
        Self {
            status: "noop".to_string(),
            reason: Some(reason.to_string()),
            mode: None,
            model: None,
            branches: Vec::new(),
            leaves_processed: 0,
            leaves_skipped: Vec::new(),
            notifications,
            warnings: Vec::new(),
        }
    }
}

/// Outcome of a compile run: the typed result plus the stderr-bound diagnostic
/// lines accumulated along the way. On success the lines also live on
/// [`CompileResult::warnings`]; on failure they ride here so the CLI can render
/// them (e.g. title-collision warnings that preceded a validation error) before
/// the error itself. The pipeline never prints — the CLI renders post-run.
#[derive(Debug)]
pub struct CompileOutcome {
    pub result: Result<CompileResult, CompileError>,
    pub(super) warnings: Vec<String>,
}

impl CompileOutcome {
    /// Stderr-bound lines, present on both the `Ok` and `Err` paths.
    pub fn stderr_lines(&self) -> &[String] {
        match &self.result {
            Ok(result) => &result.warnings,
            Err(_) => &self.warnings,
        }
    }
}

// ── dry-run preview ──────────────────────────────────────────────────────────

/// A validated, read-only compile preview. Both `--dry-run` paths produce
/// this contract: one-shot and agent. No bytes were written to produce it.
#[derive(Debug, Clone, Serialize)]
pub struct CompilePreview {
    /// "preview" when a plan was validated, "noop" when there was nothing to
    /// compile (empty tree, single leaf, no new leaves).
    pub status: String,
    pub reason: Option<String>,
    pub mode: Option<CompileRunMode>,
    pub provider: String,
    pub model: String,
    pub starting_manifest_hash: String,
    pub manifest_unchanged: bool,
    pub agent: bool,
    pub turns: usize,
    pub tool_calls: usize,
    pub usage: Option<Usage>,
    pub branches: Vec<PreviewBranch>,
    pub leaves_processed: usize,
    pub leaves_skipped: Vec<String>,
    #[serde(skip)]
    pub notifications: Vec<String>,
    #[serde(skip)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PreviewBranch {
    pub slug: String,
    pub title: String,
    pub body: String,
    pub leaves: Vec<String>,
}

/// Outcome of a dry-run: the typed preview plus stderr-bound diagnostics.
/// Mirrors `CompileOutcome` so the CLI renders both through the same shape.
#[derive(Debug)]
pub struct CompileDryRunOutcome {
    pub result: Result<CompilePreview, CompileError>,
    pub(super) warnings: Vec<String>,
}

impl CompileDryRunOutcome {
    pub fn stderr_lines(&self) -> &[String] {
        match &self.result {
            Ok(preview) => &preview.warnings,
            Err(_) => &self.warnings,
        }
    }
}

pub struct CompileSummary {
    pub branches: Vec<BranchResult>,
    pub branches_created: Vec<BranchResult>,
    pub branches_updated: Vec<BranchResult>,
    pub branch_deletes: Vec<String>,
    pub leaves_processed: usize,
    pub leaves_skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchResult {
    pub slug: String,
    pub title: String,
    pub leaf_count: usize,
}

/// Two-stage compile telemetry, journaled in the compile event payload.
#[derive(Debug, Clone, Serialize)]
pub(super) struct CompileStages {
    pub(super) stage1_clusters: usize,
    pub(super) stage2_calls: usize,
}

pub fn result_warnings(result: &CompileResult) -> Vec<JsonWarning> {
    let mut warnings = Vec::new();

    if !result.leaves_skipped.is_empty() {
        warnings.push(JsonWarning::with_details(
            "skipped_leaves",
            format!(
                "skipped {} leaves with unparseable frontmatter",
                result.leaves_skipped.len()
            ),
            json!({ "files": result.leaves_skipped }),
        ));
    }

    if let Some(msg) =
        super::degenerate_result_warning(result.mode, &result.branches, result.leaves_processed)
    {
        warnings.push(JsonWarning::new("degenerate_result", msg));
    }

    warnings
}

pub fn preview_warnings(preview: &CompilePreview) -> Vec<JsonWarning> {
    let mut warnings = Vec::new();
    if !preview.leaves_skipped.is_empty() {
        warnings.push(JsonWarning::with_details(
            "skipped_leaves",
            format!(
                "skipped {} leaves with unparseable frontmatter",
                preview.leaves_skipped.len()
            ),
            json!({ "files": preview.leaves_skipped }),
        ));
    }
    warnings
}

#[cfg(test)]
#[path = "../../tests/cli_compile_types_tests.rs"]
mod types_tests;
