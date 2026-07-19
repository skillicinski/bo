// ── synthesis types: options, outcomes, errors, constants ────────────────────

use std::time::Duration;

use serde::Serialize;
use serde_json::json;

use crate::cli::json::{JsonError, JsonWarning};
use crate::engine::llm::{LlmCallPolicy, Model, Usage};
use crate::engine::transaction;

// ── constants ─────────────────────────────────────────────────────────────────

pub(super) const MAX_COMPLETION_TOKENS: u32 = 16384;
pub(super) const MAX_SYNTHESIZED_BODY_BYTES_MIN: usize = 16 * 1024;
pub(super) const MAX_SYNTHESIZED_BODY_BYTES_PER_INPUT_BYTE: usize = 8;
pub(super) const SYNTHESIS_PROMPT_OVERHEAD_TOKENS: usize = 4096;
pub(super) const TOKEN_ESTIMATE_BYTES_PER_TOKEN: usize = 4;
pub(super) const NO_NEW_LEAVES_REASON: &str = "no new leaves since last synthesis";
const SYNTHESIS_MODEL_NEXT_STEPS: [&str; 2] = [
    "bo config --synthesis-model gpt-4.1-mini",
    "bo config --synthesis-model gpt-4.1",
];

pub const VALIDATION_NEXT_STEP: &str = "No files were changed. Try `bo synthesize` again; if this repeats, switch models with `bo config --model <model>` or report the validation message.";

pub(super) const SYNTHESIS_LLM_POLICY: LlmCallPolicy = LlmCallPolicy {
    timeout: Duration::from_secs(180),
    max_attempts: 3,
    initial_backoff: Duration::from_secs(2),
};

/// Leaf count threshold above which Full mode switches to two-stage synthesis.
pub(super) const TWO_STAGE_FULL_THRESHOLD: usize = 40;

/// New-leaf count threshold above which Incremental mode switches to two-stage.
pub(super) const TWO_STAGE_INCREMENTAL_THRESHOLD: usize = 15;

// ── errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SynthesisError {
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
    /// I/O, state, or transaction error.
    Io(String),
    /// Another bo process is mutating this tree.
    Busy(String),
    /// Validation error in the LLM response.
    Validation(String),
    /// A dry-run was blocked because recovery/repair would be required, or the
    /// state changed mid-run. Zero bytes were written.
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

impl std::fmt::Display for SynthesisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SynthesisError::ContextOverflow { model, .. } => write!(
                f,
                "synthesis model context is too small for '{}' — set a larger synthesis model, for example:\n{}\n{}",
                model, SYNTHESIS_MODEL_NEXT_STEPS[0], SYNTHESIS_MODEL_NEXT_STEPS[1]
            ),
            SynthesisError::Truncated => write!(
                f,
                "synthesis output was truncated — try reducing collection size or \
                 using a model with larger output capacity"
            ),
            SynthesisError::ContentFilter => write!(f, "synthesis was blocked by content filter"),
            SynthesisError::Llm(msg) => write!(f, "LLM error: {}", msg),
            SynthesisError::Io(msg) => write!(f, "{}", msg),
            SynthesisError::Busy(msg) => write!(f, "{}", msg),
            SynthesisError::Validation(msg) => write!(f, "{}\n{}", msg, VALIDATION_NEXT_STEP),
            SynthesisError::DryRunBlocked(msg) => write!(f, "{}", msg),
            SynthesisError::AgentFailed { message, .. } => write!(f, "{}", message),
        }
    }
}

impl From<transaction::TransactionError> for SynthesisError {
    fn from(error: transaction::TransactionError) -> Self {
        match error {
            transaction::TransactionError::Busy { .. } => SynthesisError::Busy(error.to_string()),
            other => SynthesisError::Io(other.to_string()),
        }
    }
}

impl SynthesisError {
    pub fn json_error(&self) -> JsonError {
        match self {
            SynthesisError::ContextOverflow {
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
                    "next_steps": SYNTHESIS_MODEL_NEXT_STEPS,
                }),
            ),
            SynthesisError::Truncated => JsonError::new("truncated", self.to_string()),
            SynthesisError::ContentFilter => JsonError::new("content_filter", self.to_string()),
            SynthesisError::Llm(_) => JsonError::new("llm_error", self.to_string()),
            SynthesisError::Io(_) => JsonError::new("io_error", self.to_string()),
            SynthesisError::Busy(_) => JsonError::new("tree_busy", self.to_string()),
            SynthesisError::Validation(message) => JsonError::with_details(
                "validation_error",
                message.clone(),
                json!({
                    "phase": "synthesis_validation",
                    "files_changed": false,
                    "next_step": VALIDATION_NEXT_STEP,
                }),
            ),
            SynthesisError::DryRunBlocked(message) => JsonError::with_details(
                "dry_run_blocked",
                message.clone(),
                json!({ "files_changed": false }),
            ),
            SynthesisError::AgentFailed {
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
pub struct SynthesisOptions {
    pub all: bool,
    /// Use the iterative agent path. Requires `dry_run` in this milestone.
    pub agent: bool,
    /// Produce a read-only validated preview; write zero bytes to the tree.
    pub dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SynthesisMode {
    Incremental,
    Full,
}

#[derive(Debug, Clone, Serialize)]
pub struct SynthesisResult {
    pub status: String,
    pub reason: Option<String>,
    pub mode: Option<SynthesisMode>,
    pub model: Option<String>,
    pub branches: Vec<BranchResult>,
    pub leaves_processed: usize,
    pub leaves_skipped: Vec<String>,
    #[serde(skip)]
    pub notifications: Vec<String>,
    /// Stderr-bound lines (title-collision warnings, transaction-recovery notices,
    /// per-branch write progress). Skipped from JSON — these are presentation,
    /// never part of the data envelope. Populated by the entry point.
    #[serde(skip)]
    pub warnings: Vec<String>,
}

impl SynthesisResult {
    pub fn synthesized(
        summary: SynthesisSummary,
        mode: SynthesisMode,
        model: &Model,
        notifications: Vec<String>,
    ) -> Self {
        Self {
            status: "synthesized".to_string(),
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

/// Outcome of a synthesis run: the typed result plus the stderr-bound diagnostic
/// lines accumulated along the way. On success the lines also live on
/// [`SynthesisResult::warnings`]; on failure they ride here so the CLI can render
/// them (e.g. title-collision warnings that preceded a validation error) before
/// the error itself. The pipeline never prints — the CLI renders post-run.
#[derive(Debug)]
pub struct SynthesisOutcome {
    pub result: Result<SynthesisResult, SynthesisError>,
    pub(super) warnings: Vec<String>,
}

impl SynthesisOutcome {
    /// Stderr-bound lines, present on both the `Ok` and `Err` paths.
    pub fn stderr_lines(&self) -> &[String] {
        match &self.result {
            Ok(result) => &result.warnings,
            Err(_) => &self.warnings,
        }
    }
}

// ── dry-run preview ──────────────────────────────────────────────────────────

/// A validated, read-only synthesis preview. Both `--dry-run` paths produce
/// this contract: one-shot and agent. No bytes were written to produce it.
#[derive(Debug, Clone, Serialize)]
pub struct SynthesisPreview {
    /// "preview" when a plan was validated, "noop" when there was nothing to
    /// synthesize (empty tree, single leaf, no new leaves).
    pub status: String,
    pub reason: Option<String>,
    pub mode: Option<SynthesisMode>,
    pub provider: String,
    pub model: String,
    pub starting_state_hash: String,
    pub state_unchanged: bool,
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
/// Mirrors `SynthesisOutcome` so the CLI renders both through the same shape.
#[derive(Debug)]
pub struct SynthesisDryRunOutcome {
    pub result: Result<SynthesisPreview, SynthesisError>,
    pub(super) warnings: Vec<String>,
}

impl SynthesisDryRunOutcome {
    pub fn stderr_lines(&self) -> &[String] {
        match &self.result {
            Ok(preview) => &preview.warnings,
            Err(_) => &self.warnings,
        }
    }
}

pub struct SynthesisSummary {
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

/// Two-stage synthesis telemetry, journaled in the synthesis event payload.
#[derive(Debug, Clone, Serialize)]
pub(super) struct SynthesisStages {
    pub(super) stage1_clusters: usize,
    pub(super) stage2_calls: usize,
}

pub fn result_warnings(result: &SynthesisResult) -> Vec<JsonWarning> {
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

pub fn preview_warnings(preview: &SynthesisPreview) -> Vec<JsonWarning> {
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
#[path = "../../tests/cli_synthesize_types_tests.rs"]
mod types_tests;
