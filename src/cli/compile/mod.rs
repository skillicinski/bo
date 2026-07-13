// bo compile — deterministic pipeline with a single structured LLM call.
//
// Pipeline: read leaves → build prompt → LLM call → parse/validate → write → summary
//
// No agent loop, no tool dispatch. The LLM receives all leaf content and returns
// a structured JSON response with identified concepts (branches) and their
// leaf associations.

use std::collections::HashSet;
use std::time::Duration;

use serde::Serialize;
use serde_json::json;

use crate::cli::json::JsonError;
use crate::domain::tree::{self, TreeRuntimeState};
use crate::domain::{manifest, Timestamp};
use crate::engine::auth;
use crate::engine::config::SeededConfig;
use crate::engine::llm::{LlmCallPolicy, LlmProvider, Model, Usage};
use crate::engine::pending;

mod agent;
mod execute;
mod parse;
mod plan;
mod prompt;
mod render;
mod repair;
mod validation;

// ── constants ─────────────────────────────────────────────────────────────────

const MAX_COMPLETION_TOKENS: u32 = 16384;
const MAX_COMPILED_BODY_BYTES_MIN: usize = 16 * 1024;
const MAX_COMPILED_BODY_BYTES_PER_INPUT_BYTE: usize = 8;
const COMPILE_PROMPT_OVERHEAD_TOKENS: usize = 4096;
const TOKEN_ESTIMATE_BYTES_PER_TOKEN: usize = 4;
const NO_NEW_LEAVES_REASON: &str = "no new leaves since last compile";
const COMPILE_MODEL_NEXT_STEPS: [&str; 2] = [
    "bo config --compile-model gpt-4.1-mini",
    "bo config --compile-model gpt-4.1",
];

pub const VALIDATION_NEXT_STEP: &str = "No files were changed. Try `bo compile` again; if this repeats, switch models with `bo config --model <model>` or report the validation message.";

const COMPILE_LLM_POLICY: LlmCallPolicy = LlmCallPolicy {
    timeout: Duration::from_secs(180),
    max_attempts: 3,
    initial_backoff: Duration::from_secs(2),
};

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
    /// JSON carries turns/tool_calls/usage/last_error.
    AgentFailed {
        message: String,
        turns: usize,
        tool_calls: usize,
        usage: Option<Usage>,
        last_error: Option<String>,
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
            } => JsonError::with_details(
                "agent_error",
                message.clone(),
                json!({
                    "files_changed": false,
                    "turns": turns,
                    "tool_calls": tool_calls,
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
    fn compiled(
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

    fn noop(reason: &str, notifications: Vec<String>) -> Self {
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
    warnings: Vec<String>,
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
    warnings: Vec<String>,
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
    pub leaves_processed: usize,
    pub leaves_skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchResult {
    pub slug: String,
    pub title: String,
    pub leaf_count: usize,
}

fn preflight_noop(
    manifest: &manifest::Manifest,
    options: CompileOptions,
    notifications: &[String],
) -> Option<CompileResult> {
    match manifest.leaves.len() {
        0 => return Some(CompileResult::noop("empty_tree", notifications.to_vec())),
        1 => return Some(CompileResult::noop("single_leaf", notifications.to_vec())),
        _ => {}
    }

    let _ = options;
    None
}

pub fn run_compile_with_options(cfg: &SeededConfig, options: CompileOptions) -> CompileOutcome {
    let mut warnings = Vec::new();
    let result = run_compile(cfg, options, &mut warnings);
    match result {
        Ok(mut result) => {
            result.warnings = warnings;
            CompileOutcome {
                result: Ok(result),
                warnings: Vec::new(),
            }
        }
        Err(error) => CompileOutcome {
            result: Err(error),
            warnings,
        },
    }
}

// ── dry-run ────────────────────────────────────────────────────────────────
//
// Two phases: a read-only preflight (pending/stale detection, manifest read,
// noop checks) that needs no provider, and a plan-building phase that resolves
// the provider only when an LLM call is actually required. Noop paths therefore
// work without an API key, matching `bo compile` on an empty tree.

struct DryRunRequest {
    manifest: manifest::Manifest,
    loaded_leaves: Vec<plan::LoadedLeaf>,
    skipped_leaves: Vec<String>,
    new_leaf_slugs: Vec<String>,
    run_mode: CompileRunMode,
    starting_hash: String,
}

enum DryRunPreflight {
    Noop(CompilePreview),
    NeedsLlm(DryRunRequest),
}

/// Public dry-run entry point. Resolves the provider lazily — only when an
/// LLM call is needed. Zero tree writes in every path.
pub fn run_compile_dry_run(cfg: &SeededConfig, options: CompileOptions) -> CompileDryRunOutcome {
    let mut warnings = Vec::new();
    let preflight = dry_run_preflight(cfg, options, &mut warnings);
    let preflight = match preflight {
        Ok(DryRunPreflight::Noop(preview)) => {
            return CompileDryRunOutcome {
                result: Ok(preview),
                warnings,
            }
        }
        Ok(DryRunPreflight::NeedsLlm(req)) => req,
        Err(error) => {
            return CompileDryRunOutcome {
                result: Err(error),
                warnings,
            }
        }
    };

    let result = (|| -> Result<CompilePreview, CompileError> {
        let api_key = auth::resolve_api_key(cfg.config.provider)
            .map_err(|e| CompileError::Llm(e.to_string()))?;
        let provider = crate::engine::llm::create_provider(cfg.config.provider, &api_key)
            .map_err(|e| CompileError::Llm(e.to_string()))?;
        let model = cfg
            .config
            .effective_compile_model()
            .map_err(|e| CompileError::Llm(e.to_string()))?;
        dry_run_build_plan(
            cfg,
            options,
            provider.as_ref(),
            &model,
            preflight,
            &mut warnings,
        )
    })();
    match result {
        Ok(mut preview) => {
            preview.warnings = warnings;
            CompileDryRunOutcome {
                result: Ok(preview),
                warnings: Vec::new(),
            }
        }
        Err(error) => CompileDryRunOutcome {
            result: Err(error),
            warnings,
        },
    }
}

/// Testable dry-run seam with an injected provider and model. Same zero-write
/// preflight; the caller supplies the provider so scripted tests can drive the
/// agent loop deterministically.
pub fn run_compile_dry_run_with_provider(
    cfg: &SeededConfig,
    options: CompileOptions,
    provider: &dyn LlmProvider,
    model: &Model,
) -> CompileDryRunOutcome {
    let mut warnings = Vec::new();
    let preflight = dry_run_preflight(cfg, options, &mut warnings);
    let preflight = match preflight {
        Ok(DryRunPreflight::Noop(preview)) => {
            return CompileDryRunOutcome {
                result: Ok(preview),
                warnings,
            }
        }
        Ok(DryRunPreflight::NeedsLlm(req)) => req,
        Err(error) => {
            return CompileDryRunOutcome {
                result: Err(error),
                warnings,
            }
        }
    };
    let result = dry_run_build_plan(cfg, options, provider, model, preflight, &mut warnings);
    match result {
        Ok(mut preview) => {
            preview.warnings = warnings;
            CompileDryRunOutcome {
                result: Ok(preview),
                warnings: Vec::new(),
            }
        }
        Err(error) => CompileDryRunOutcome {
            result: Err(error),
            warnings,
        },
    }
}

fn dry_run_preflight(
    cfg: &SeededConfig,
    options: CompileOptions,
    _warnings: &mut Vec<String>,
) -> Result<DryRunPreflight, CompileError> {
    let tree = cfg.tree();
    let tree_dir = tree.path();

    // ZERO writes: read-only pending check. Do not recover.
    if pending::read(&pending::pending_path(tree_dir))?.is_some() {
        return Err(CompileError::DryRunBlocked(
            "a pending operation exists; run `bo compile` (without --dry-run) to recover it before previewing".to_string(),
        ));
    }

    let manifest = match crate::engine::manifest::runtime_state(tree_dir) {
        Ok(TreeRuntimeState::Initialized(manifest)) => manifest,
        Ok(TreeRuntimeState::FreshSeeded) => {
            return Ok(DryRunPreflight::Noop(noop_preview(
                "empty_tree",
                cfg,
                "<missing>",
                options.agent,
            )));
        }
        Ok(TreeRuntimeState::MissingManifest) => {
            return Err(CompileError::Io(format!(
                "failed to read manifest: {}",
                manifest::ManifestError::TreeNotInitialized
            )));
        }
        Err(error) => {
            return Err(CompileError::Io(format!(
                "failed to read manifest: {}",
                error
            )));
        }
    };

    // ZERO writes: read-only stale-repair check. Do not repair.
    if repair::requires_repair(cfg, &manifest)? {
        return Err(CompileError::DryRunBlocked(
            "stale branches require repair; run `bo compile` (without --dry-run) to repair before previewing".to_string(),
        ));
    }

    // Capture the manifest hash at start; recheck before accepting the preview.
    let starting_hash = pending::manifest_hash(tree_dir)?;

    let run_mode = plan::select_run_mode(options, &manifest);
    if manifest.leaves.is_empty() {
        return Ok(DryRunPreflight::Noop(noop_preview(
            "empty_tree",
            cfg,
            &starting_hash,
            options.agent,
        )));
    }
    let new_leaf_slugs = plan::select_new_leaf_slugs(&manifest)?;
    if !options.all && new_leaf_slugs.is_empty() {
        return Ok(DryRunPreflight::Noop(noop_preview(
            NO_NEW_LEAVES_REASON,
            cfg,
            &starting_hash,
            options.agent,
        )));
    }
    let (loaded_leaves, skipped_leaves) = plan::read_valid_leaves(cfg, &manifest.leaves);
    if loaded_leaves.is_empty() {
        return Ok(DryRunPreflight::Noop(noop_preview(
            "empty_tree",
            cfg,
            &starting_hash,
            options.agent,
        )));
    }
    if loaded_leaves.len() < 2 {
        return Ok(DryRunPreflight::Noop(noop_preview(
            "single_leaf",
            cfg,
            &starting_hash,
            options.agent,
        )));
    }

    Ok(DryRunPreflight::NeedsLlm(DryRunRequest {
        manifest,
        loaded_leaves,
        skipped_leaves,
        new_leaf_slugs,
        run_mode,
        starting_hash,
    }))
}

fn dry_run_build_plan(
    cfg: &SeededConfig,
    options: CompileOptions,
    provider: &dyn LlmProvider,
    model: &Model,
    req: DryRunRequest,
    warnings: &mut Vec<String>,
) -> Result<CompilePreview, CompileError> {
    let DryRunRequest {
        manifest,
        loaded_leaves,
        skipped_leaves,
        new_leaf_slugs,
        run_mode,
        starting_hash,
    } = req;

    let (plan, stats, validation_warnings) = if options.agent {
        let (plan, stats, vw) =
            agent::run_agent_dry_run(cfg, provider, model, &manifest, &loaded_leaves, run_mode)?;
        (plan, stats, vw)
    } else {
        let (plan, stats) = run_one_shot_dry_run(
            cfg,
            provider,
            model,
            &manifest,
            &loaded_leaves,
            &new_leaf_slugs,
            run_mode,
            warnings,
        )?;
        (plan, stats, Vec::new())
    };
    warnings.extend(validation_warnings);

    // Recheck the manifest hash; abort if the tree changed mid-run.
    let current_hash = pending::manifest_hash(cfg.tree().path())?;
    let manifest_unchanged = current_hash == starting_hash;
    if !manifest_unchanged {
        return Err(CompileError::DryRunBlocked(
            "manifest changed during dry-run; rerun `bo compile --dry-run`".to_string(),
        ));
    }

    Ok(CompilePreview {
        status: "preview".to_string(),
        reason: None,
        mode: Some(run_mode),
        provider: cfg.config.provider.to_string(),
        model: model.to_string(),
        starting_manifest_hash: starting_hash,
        manifest_unchanged,
        agent: options.agent,
        turns: stats.turns,
        tool_calls: stats.tool_calls,
        usage: stats.usage,
        branches: plan
            .branches
            .iter()
            .map(|b| PreviewBranch {
                slug: b.slug.clone(),
                title: b.title.clone(),
                body: b.body.clone(),
                leaves: b.leaves.clone(),
            })
            .collect(),
        leaves_processed: loaded_leaves.len(),
        leaves_skipped: skipped_leaves,
        notifications: Vec::new(),
        warnings: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn run_one_shot_dry_run(
    cfg: &SeededConfig,
    provider: &dyn LlmProvider,
    model: &Model,
    manifest: &manifest::Manifest,
    loaded_leaves: &[plan::LoadedLeaf],
    new_leaf_slugs: &[String],
    run_mode: CompileRunMode,
    warnings: &mut Vec<String>,
) -> Result<(validation::CompilePlan, agent::AgentRunStats), CompileError> {
    let (user_message, prompt_tokens, schema) = match run_mode {
        CompileRunMode::Full => {
            let msg = prompt::build_user_message(loaded_leaves);
            let tokens = execute::estimate_compile_prompt_tokens(
                prompt::COMPILE_SYSTEM_PROMPT
                    .len()
                    .saturating_add(msg.len()),
            );
            (
                msg,
                tokens,
                serde_json::to_value(crate::engine::schema::inline_schema_for::<
                    parse::CompileResponse,
                >())
                .unwrap(),
            )
        }
        CompileRunMode::Incremental => {
            let msg = prompt::build_incremental_user_message(
                cfg,
                manifest,
                loaded_leaves,
                new_leaf_slugs,
            );
            let tokens = execute::estimate_compile_prompt_tokens(
                prompt::COMPILE_SYSTEM_PROMPT
                    .len()
                    .saturating_add(msg.len()),
            );
            (
                msg,
                tokens,
                serde_json::to_value(crate::engine::schema::inline_schema_for::<
                    parse::IncrementalCompileResponse,
                >())
                .unwrap(),
            )
        }
    };
    execute::ensure_compile_context_fits(model, prompt_tokens)?;
    let response = execute::call_llm_blocking(provider, model, &user_message, &schema)?;
    let input_body_bytes: usize = loaded_leaves.iter().map(|l| l.body.len()).sum();
    let plan = match run_mode {
        CompileRunMode::Full => parse::parse_and_validate_with_input_size(
            &response,
            loaded_leaves,
            input_body_bytes,
            warnings,
        )?,
        CompileRunMode::Incremental => parse::parse_and_validate_incremental_with_input_size(
            &response,
            cfg,
            loaded_leaves,
            input_body_bytes,
            warnings,
        )?,
    };
    Ok((
        plan,
        agent::AgentRunStats {
            turns: 1,
            tool_calls: 0,
            usage: None,
        },
    ))
}

fn noop_preview(
    reason: &str,
    cfg: &SeededConfig,
    starting_hash: &str,
    is_agent: bool,
) -> CompilePreview {
    let model_str = cfg
        .config
        .compile_model
        .as_deref()
        .unwrap_or(&cfg.config.model);
    CompilePreview {
        status: "noop".to_string(),
        reason: Some(reason.to_string()),
        mode: None,
        provider: cfg.config.provider.to_string(),
        model: model_str.to_string(),
        starting_manifest_hash: starting_hash.to_string(),
        manifest_unchanged: true,
        agent: is_agent,
        turns: 0,
        tool_calls: 0,
        usage: None,
        branches: Vec::new(),
        leaves_processed: 0,
        leaves_skipped: Vec::new(),
        notifications: Vec::new(),
        warnings: Vec::new(),
    }
}

fn run_compile(
    cfg: &SeededConfig,
    options: CompileOptions,
    warnings: &mut Vec<String>,
) -> Result<CompileResult, CompileError> {
    let compile_started_at = Timestamp::now();

    // Stale repair runs before preflight so preflight sees repaired state.
    let tree = cfg.tree();
    execute::recover_pending_if_needed(tree.path(), warnings)?;
    let manifest = match crate::engine::manifest::runtime_state(tree.path()) {
        Ok(TreeRuntimeState::Initialized(manifest)) => manifest,
        Ok(TreeRuntimeState::FreshSeeded) => {
            return Ok(CompileResult::noop("empty_tree", Vec::new()));
        }
        Ok(TreeRuntimeState::MissingManifest) => {
            return Err(CompileError::Io(format!(
                "failed to read manifest: {}",
                manifest::ManifestError::TreeNotInitialized
            )));
        }
        Err(error) => {
            return Err(CompileError::Io(format!(
                "failed to read manifest: {}",
                error
            )));
        }
    };
    let notifications = repair::repair_stale_branches(cfg, &manifest)?;
    // Repair notices are destructive-action reporting (e.g. "removed N stale
    // branches"); mirror them onto the stderr channel so they reach consumers
    // in both human and --json mode. The human-mode double-emission (stderr
    // line + stdout `→` note) matches the prior behavior byte-for-byte.
    warnings.extend(notifications.iter().cloned());
    let manifest = crate::engine::manifest::read(&tree::manifest_path(tree.path()))
        .map_err(|e| CompileError::Io(format!("failed to read manifest: {}", e)))?;

    if let Some(noop) = preflight_noop(&manifest, options, &notifications) {
        return Ok(noop);
    }
    let new_leaf_slugs = plan::select_new_leaf_slugs(&manifest)?;
    if !options.all && new_leaf_slugs.is_empty() {
        return Ok(CompileResult::noop(NO_NEW_LEAVES_REASON, notifications));
    }

    let expected_manifest_hash = pending::manifest_hash(tree.path())?;

    let api_key =
        auth::resolve_api_key(cfg.config.provider).map_err(|e| CompileError::Llm(e.to_string()))?;
    let provider = crate::engine::llm::create_provider(cfg.config.provider, &api_key)
        .map_err(|e| CompileError::Llm(e.to_string()))?;
    let compile_model = cfg
        .config
        .effective_compile_model()
        .map_err(|e| CompileError::Llm(e.to_string()))?;
    run_compile_with_provider_started_at(
        cfg,
        options,
        provider.as_ref(),
        &compile_model,
        &compile_started_at,
        notifications,
        &manifest,
        &new_leaf_slugs,
        &expected_manifest_hash,
        warnings,
    )
}

// ponytail: 10 args; collapse into a preflight struct if it grows further.
#[allow(clippy::too_many_arguments)]
pub fn run_compile_with_provider_started_at(
    cfg: &SeededConfig,
    options: CompileOptions,
    provider: &dyn LlmProvider,
    model: &Model,
    compile_started_at: &crate::domain::Timestamp,
    mut notifications: Vec<String>,
    manifest: &manifest::Manifest,
    new_leaf_slugs: &[String],
    expected_manifest_hash: &str,
    warnings: &mut Vec<String>,
) -> Result<CompileResult, CompileError> {
    let (loaded_leaves, skipped_leaves) = plan::read_valid_leaves(cfg, &manifest.leaves);

    if loaded_leaves.len() < 2 {
        if loaded_leaves.is_empty() {
            return Ok(CompileResult::noop("empty_tree", notifications));
        }
        return Ok(CompileResult::noop("single_leaf", notifications));
    }

    // ── build prompt and schema ─────────────────────────────────────────────
    // Each run mode has exactly one coherent prompt and schema. Incremental
    // mode is only chosen when branches exist (see select_run_mode) and always
    // sends the existing branch graph; Full mode sends all leaf bodies.
    let run_mode = plan::select_run_mode(options, manifest);

    let (user_message, prompt_tokens, response_schema) = match run_mode {
        CompileRunMode::Full => {
            let msg = prompt::build_user_message(&loaded_leaves);
            let tokens = execute::estimate_compile_prompt_tokens(
                prompt::COMPILE_SYSTEM_PROMPT
                    .len()
                    .saturating_add(msg.len()),
            );
            (
                msg,
                tokens,
                serde_json::to_value(crate::engine::schema::inline_schema_for::<
                    parse::CompileResponse,
                >())
                .unwrap(),
            )
        }
        CompileRunMode::Incremental => {
            let msg = prompt::build_incremental_user_message(
                cfg,
                manifest,
                &loaded_leaves,
                new_leaf_slugs,
            );
            let tokens = execute::estimate_compile_prompt_tokens(
                prompt::COMPILE_SYSTEM_PROMPT
                    .len()
                    .saturating_add(msg.len()),
            );
            (
                msg,
                tokens,
                serde_json::to_value(crate::engine::schema::inline_schema_for::<
                    parse::IncrementalCompileResponse,
                >())
                .unwrap(),
            )
        }
    };
    execute::ensure_compile_context_fits(model, prompt_tokens)?;

    // ── LLM call ─────────────────────────────────────────────────────────────
    let response = execute::call_llm_blocking(provider, model, &user_message, &response_schema)?;

    // ── parse and validate ───────────────────────────────────────────────────
    let valid_filenames: HashSet<String> =
        loaded_leaves.iter().map(|l| l.filename.clone()).collect();
    let input_body_bytes = loaded_leaves.iter().map(|l| l.body.len()).sum();

    // ── execute validated plan ───────────────────────────────────────────────
    let run_timestamp = compile_started_at;
    let compiled_plan = match run_mode {
        CompileRunMode::Full => parse::parse_and_validate_with_input_size(
            &response,
            &loaded_leaves,
            input_body_bytes,
            warnings,
        )?,
        CompileRunMode::Incremental => parse::parse_and_validate_incremental_with_input_size(
            &response,
            cfg,
            &loaded_leaves,
            input_body_bytes,
            warnings,
        )?,
    };

    let summary = execute::execute_plan_with_mode_and_expected_hash(
        &compiled_plan,
        cfg,
        &valid_filenames,
        run_timestamp,
        &skipped_leaves,
        run_mode,
        expected_manifest_hash,
        warnings,
    )?;

    if let Some(warning) =
        degenerate_result_warning(Some(run_mode), &summary.branches, summary.leaves_processed)
    {
        notifications.push(warning);
    }

    Ok(CompileResult::compiled(
        summary,
        run_mode,
        model,
        notifications,
    ))
}

/// Check for a degenerate Full compile result (quality collapse):
/// returns a warning string when the branch-to-leaf ratio is implausibly low.
/// Only applies to Full mode; Incremental mode naturally produces fewer branches.
pub fn degenerate_result_warning(
    mode: Option<CompileRunMode>,
    branches: &[BranchResult],
    leaves_processed: usize,
) -> Option<String> {
    if mode != Some(CompileRunMode::Full) || leaves_processed <= 20 {
        return None;
    }
    // ponytail: shared remediation hint; both failure modes suggest the same fix.
    const FIX: &str = "the model likely collapsed; try `bo compile --all` again or switch models with `bo config --compile-model <model>`";
    let branch_count = branches.len();
    if branch_count < 2 {
        return Some(format!(
            "degenerate compile result: {} branch(es) for {} leaves — {FIX}",
            branch_count, leaves_processed
        ));
    }
    let branched_leaf_count: usize = branches.iter().map(|b| b.leaf_count).sum();
    let unbranched = leaves_processed.saturating_sub(branched_leaf_count);
    let unbranched_pct = (unbranched as f64 / leaves_processed as f64) * 100.0;
    if unbranched_pct > 80.0 {
        return Some(format!(
            "degenerate compile result: {} of {} leaves unbranched ({:.0}%) — {FIX}",
            unbranched, leaves_processed, unbranched_pct
        ));
    }
    // ponytail: 0.30 threshold is a guess calibrated against the 15/66≈0.23
    // dogfooding case; re-tune against more real-world degenerate results.
    let coverage = branched_leaf_count as f64 / leaves_processed as f64;
    if coverage < 0.30 {
        return Some(format!(
            "degenerate compile result: only {} of {} leaves placed in branches — {FIX}",
            branched_leaf_count, leaves_processed
        ));
    }
    None
}

pub fn render_human<W: std::io::Write>(
    result: &CompileResult,
    stdout: &mut W,
    tree_name: &str,
) -> std::io::Result<()> {
    render::render_human(result, stdout, tree_name)
}

/// Render a dry-run preview to stdout (human mode).
pub fn render_preview_human<W: std::io::Write>(
    preview: &CompilePreview,
    stdout: &mut W,
    tree_name: &str,
) -> std::io::Result<()> {
    render::render_preview_human(preview, stdout, tree_name)
}

/// Render the stderr-bound diagnostic lines collected during a compile run.
/// Called by the CLI post-run; nothing below the entry point prints.
pub fn render_diagnostics<W: std::io::Write>(
    lines: &[String],
    stderr: &mut W,
) -> std::io::Result<()> {
    render::render_diagnostics(lines, stderr)
}

#[cfg(test)]
#[path = "../../tests/cli_compile_tests.rs"]
mod tests;
