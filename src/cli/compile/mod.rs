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
use crate::engine::llm::{LlmCallPolicy, LlmProvider, Model};
use crate::engine::pending;

mod execute;
mod parse;
mod plan;
mod prompt;
mod render;
mod schema;

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
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::ContextOverflow { model, .. } => write!(
                f,
                "compile model context is too small for '{}' — set a larger compile model, for example:\n{}\n{}",
                model,
                COMPILE_MODEL_NEXT_STEPS[0],
                COMPILE_MODEL_NEXT_STEPS[1]
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
        }
    }
}

// ── public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompileOptions {
    pub all: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompileRunMode {
    Incremental,
    Full,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompileContextMode {
    FullCorpus,
    IncrementalContext,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompileResult {
    pub status: String,
    pub reason: Option<String>,
    pub mode: Option<CompileRunMode>,
    pub context_mode: Option<CompileContextMode>,
    pub model: Option<String>,
    pub branches: Vec<BranchResult>,
    pub leaves_processed: usize,
    pub leaves_skipped: Vec<String>,
    #[serde(skip)]
    pub notifications: Vec<String>,
}

impl CompileResult {
    fn compiled(
        summary: CompileSummary,
        mode: CompileRunMode,
        context_mode: CompileContextMode,
        model: &Model,
        notifications: Vec<String>,
    ) -> Self {
        Self {
            status: "compiled".to_string(),
            reason: None,
            mode: Some(mode),
            context_mode: Some(context_mode),
            model: Some(model.to_string()),
            branches: summary.branches,
            leaves_processed: summary.leaves_processed,
            leaves_skipped: summary.leaves_skipped,
            notifications,
        }
    }

    fn noop(reason: &str, notifications: Vec<String>) -> Self {
        Self {
            status: "noop".to_string(),
            reason: Some(reason.to_string()),
            mode: None,
            context_mode: None,
            model: None,
            branches: Vec::new(),
            leaves_processed: 0,
            leaves_skipped: Vec::new(),
            notifications,
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

pub fn run_compile_with_options(
    cfg: &SeededConfig,
    options: CompileOptions,
) -> Result<CompileResult, CompileError> {
    let compile_started_at = Timestamp::now();

    // Stale repair runs before preflight so preflight sees repaired state.
    let tree = cfg.tree();
    execute::recover_pending_if_needed(tree.path())?;
    let manifest = match tree.runtime_state() {
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
    let notifications = plan::repair_stale_branches(cfg, &manifest)?;
    let manifest = manifest::read(&tree::manifest_path(tree.path()))
        .map_err(|e| CompileError::Io(format!("failed to read manifest: {}", e)))?;

    if let Some(noop) = preflight_noop(&manifest, options, &notifications) {
        return Ok(noop);
    }
    let new_leaf_slugs = plan::select_new_leaf_slugs(&manifest)?;
    if !options.all && new_leaf_slugs.is_empty() {
        return Ok(CompileResult::noop(NO_NEW_LEAVES_REASON, notifications));
    }

    let api_key =
        auth::resolve_api_key(cfg.config.provider).map_err(|e| CompileError::Llm(e.to_string()))?;
    let provider = crate::engine::llm::create_provider(cfg.config.provider, &api_key);
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
    )
}

fn run_compile_with_provider_started_at(
    cfg: &SeededConfig,
    options: CompileOptions,
    provider: &dyn LlmProvider,
    model: &Model,
    compile_started_at: &crate::domain::Timestamp,
    notifications: Vec<String>,
) -> Result<CompileResult, CompileError> {
    let tree = cfg.tree();
    execute::recover_pending_if_needed(tree.path())?;

    let manifest = manifest::read(&tree::manifest_path(tree.path()))
        .map_err(|e| CompileError::Io(format!("failed to read manifest: {}", e)))?;
    let expected_manifest_hash = pending::manifest_hash(tree.path())?;

    let new_leaf_slugs = plan::select_new_leaf_slugs(&manifest)?;

    if !options.all && new_leaf_slugs.is_empty() {
        return Ok(CompileResult::noop(NO_NEW_LEAVES_REASON, notifications));
    }

    let (loaded_leaves, skipped_leaves) = plan::read_valid_leaves(cfg, &manifest.leaves);

    if loaded_leaves.len() < 2 {
        if loaded_leaves.is_empty() {
            return Ok(CompileResult::noop("empty_tree", notifications));
        }
        return Ok(CompileResult::noop("single_leaf", notifications));
    }

    // ── build prompt and schema ──────────────────────────────────────────────
    let full_user_message = prompt::build_user_message(&loaded_leaves);
    let full_prompt_tokens = execute::estimate_compile_prompt_tokens(
        prompt::COMPILE_SYSTEM_PROMPT
            .len()
            .saturating_add(full_user_message.len()),
    );
    let run_mode = if options.all {
        CompileRunMode::Full
    } else {
        CompileRunMode::Incremental
    };
    let incremental_user_message =
        prompt::build_incremental_user_message(cfg, &manifest, &loaded_leaves, &new_leaf_slugs);
    let incremental_prompt_tokens = execute::estimate_compile_prompt_tokens(
        prompt::COMPILE_SYSTEM_PROMPT
            .len()
            .saturating_add(incremental_user_message.len()),
    );
    let context_mode = execute::choose_context_mode(
        model,
        run_mode,
        full_prompt_tokens,
        incremental_prompt_tokens,
    )?;
    let user_message = if context_mode == CompileContextMode::IncrementalContext {
        incremental_user_message
    } else {
        full_user_message
    };
    let response_schema = if run_mode == CompileRunMode::Incremental {
        schema::incremental_compile_response_schema()
    } else {
        schema::compile_response_schema()
    };

    // ── LLM call ─────────────────────────────────────────────────────────────
    let response = execute::call_llm_blocking(provider, model, &user_message, &response_schema)?;

    // ── parse and validate ───────────────────────────────────────────────────
    let valid_filenames: HashSet<String> =
        loaded_leaves.iter().map(|l| l.filename.clone()).collect();
    let input_body_bytes = loaded_leaves.iter().map(|l| l.body.len()).sum();

    // ── execute validated plan ───────────────────────────────────────────────
    let run_timestamp = compile_started_at;
    let compiled_plan = match run_mode {
        CompileRunMode::Full => {
            parse::parse_and_validate_with_input_size(&response, &loaded_leaves, input_body_bytes)?
        }
        CompileRunMode::Incremental => parse::parse_and_validate_incremental_with_input_size(
            &response,
            cfg,
            &loaded_leaves,
            input_body_bytes,
        )?,
    };

    let summary = execute::execute_plan_with_mode_and_expected_hash(
        &compiled_plan,
        cfg,
        &valid_filenames,
        run_timestamp,
        &skipped_leaves,
        run_mode,
        &expected_manifest_hash,
    )?;

    Ok(CompileResult::compiled(
        summary,
        run_mode,
        context_mode,
        model,
        notifications,
    ))
}

pub fn render_human<W: std::io::Write>(
    result: &CompileResult,
    stdout: &mut W,
    tree_name: &str,
) -> std::io::Result<()> {
    render::render_human(result, stdout, tree_name)
}

#[cfg(test)]
#[path = "../../tests/cli_compile_tests.rs"]
mod tests;
