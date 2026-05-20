// bo compile — deterministic pipeline with a single structured LLM call.
//
// Pipeline: read leaves → build prompt → LLM call → parse/validate → write → summary
//
// No agent loop, no tool dispatch. The LLM receives all leaf content and returns
// a structured JSON response with identified concepts (branches) and their
// leaf associations.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::domain::manifest::{self, BranchRecord, LeafRecord, Manifest, TreeMeta};
use crate::domain::{branch, frontmatter, slug, tree::Tree};
use crate::engine::auth::{self, AuthResolutionError};
use crate::engine::config::SeededConfig;
use crate::engine::llm::{
    complete_with_policy, context_window_tokens, FinishReason, LlmCallPolicy, LlmError,
    LlmProvider, Message, OpenAiProvider,
};
use crate::engine::pending::{self, CompileMode, OpKind, PendingWrite};

// ── constants ─────────────────────────────────────────────────────────────────

const MAX_COMPLETION_TOKENS: u32 = 16384;
const MAX_COMPILED_BODY_BYTES_MIN: usize = 16 * 1024;
const MAX_COMPILED_BODY_BYTES_PER_INPUT_BYTE: usize = 8;
const COMPILE_PROMPT_OVERHEAD_TOKENS: usize = 4096;
const TOKEN_ESTIMATE_BYTES_PER_TOKEN: usize = 4;
const NO_NEW_LEAVES_REASON: &str = "no new leaves since last compile";
const COMPILE_MODEL_NEXT_STEPS: [&str; 2] = [
    "bo config set compile_model gpt-4.1-mini",
    "bo config set compile_model gpt-4.1",
];

pub const VALIDATION_NEXT_STEP: &str = "No files were changed. Try `bo compile` again; if this repeats, switch models with `bo config set model <model>` or report the validation message.";

const COMPILE_LLM_POLICY: LlmCallPolicy = LlmCallPolicy {
    timeout: Duration::from_secs(180),
    max_attempts: 3,
    initial_backoff: Duration::from_secs(2),
};

fn compile_timestamp_now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn parse_rfc3339_utc(value: &str) -> Result<DateTime<Utc>, CompileError> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|error| validation_error(format!("invalid RFC3339 timestamp '{value}': {error}")))
}

fn collected_after_last_compile(
    collected_at: &str,
    last_compiled_at: &str,
) -> Result<bool, CompileError> {
    Ok(parse_rfc3339_utc(collected_at)? > parse_rfc3339_utc(last_compiled_at)?)
}

fn estimate_tokens_from_bytes(bytes: usize) -> usize {
    bytes.div_ceil(TOKEN_ESTIMATE_BYTES_PER_TOKEN)
}

fn estimate_compile_prompt_tokens(prompt_bytes: usize) -> usize {
    COMPILE_PROMPT_OVERHEAD_TOKENS
        .saturating_add(MAX_COMPLETION_TOKENS as usize)
        .saturating_add(estimate_tokens_from_bytes(prompt_bytes))
}

fn ensure_compile_context_fits(model: &str, estimated_tokens: usize) -> Result<(), CompileError> {
    let Some(context_tokens) = context_window_tokens(model) else {
        return Err(CompileError::ContextOverflow {
            model: model.to_string(),
            estimated_tokens: Some(estimated_tokens),
            context_tokens: None,
        });
    };

    if estimated_tokens > context_tokens {
        return Err(CompileError::ContextOverflow {
            model: model.to_string(),
            estimated_tokens: Some(estimated_tokens),
            context_tokens: Some(context_tokens),
        });
    }

    Ok(())
}

fn choose_context_mode(
    model: &str,
    run_mode: CompileRunMode,
    full_prompt_tokens: usize,
    incremental_prompt_tokens: usize,
) -> Result<CompileContextMode, CompileError> {
    match run_mode {
        CompileRunMode::Full => {
            ensure_compile_context_fits(model, full_prompt_tokens)?;
            Ok(CompileContextMode::FullCorpus)
        }
        CompileRunMode::Incremental => {
            if ensure_compile_context_fits(model, full_prompt_tokens).is_ok() {
                return Ok(CompileContextMode::FullCorpus);
            }
            ensure_compile_context_fits(model, incremental_prompt_tokens)?;
            Ok(CompileContextMode::IncrementalContext)
        }
    }
}

fn select_new_leaf_slugs(manifest: &Manifest) -> Result<Vec<String>, CompileError> {
    let Some(last_compiled_at) = manifest.tree.last_compiled_at.as_deref() else {
        return Ok(manifest
            .leaves
            .iter()
            .map(|leaf| leaf.slug.clone())
            .collect());
    };

    manifest
        .leaves
        .iter()
        .filter_map(|leaf| {
            match collected_after_last_compile(&leaf.collected_at, last_compiled_at) {
                Ok(true) => Some(Ok(leaf.slug.clone())),
                Ok(false) => None,
                Err(error) => Some(Err(error)),
            }
        })
        .collect()
}

fn derive_stale_branch_slugs(manifest: &Manifest, deleted_leaf_slugs: &[String]) -> Vec<String> {
    let deleted_leaf_slugs: HashSet<&str> = deleted_leaf_slugs.iter().map(String::as_str).collect();
    let mut stale_branch_slugs = Vec::new();

    for branch in &manifest.branches {
        if branch
            .leaves
            .iter()
            .any(|leaf_slug| deleted_leaf_slugs.contains(leaf_slug.as_str()))
        {
            stale_branch_slugs.push(branch.slug.clone());
        }
    }

    stale_branch_slugs
}

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

const COMPILE_SYSTEM_PROMPT: &str = "\
You are a knowledge compilation engine for a personal document collection.

Your task: identify recurring concepts and themes that appear across multiple \
documents, then produce structured output describing each concept.

## Rules

- A concept MUST appear in at least two documents. Never create a branch with only one leaf. \
  If a topic only appears in a single document, do not create a branch for it — it is not a \
  cross-cutting concept.
- Prefer specific, recurring themes over broad catch-all categories.
- Each branch body should synthesise how the concept manifests across the documents — \
  draw connections, note contrasts, highlight patterns. Do not just summarise each document \
  in turn.
- The body should begin with a single markdown heading matching the title (e.g. `# Concept Name`). \
  Do not repeat the heading or nest a second heading immediately after.
- Reference documents by their filename only when making a specific point about that document's \
  contribution to the concept.
- Only use document filenames exactly as provided in the input.
- If no cross-cutting concepts span two or more documents, return an empty branches array.
";

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
}

impl CompileResult {
    fn compiled(
        summary: CompileSummary,
        mode: CompileRunMode,
        context_mode: CompileContextMode,
        model: &str,
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
        }
    }

    fn noop(reason: &str) -> Self {
        Self {
            status: "noop".to_string(),
            reason: Some(reason.to_string()),
            mode: None,
            context_mode: None,
            model: None,
            branches: Vec::new(),
            leaves_processed: 0,
            leaves_skipped: Vec::new(),
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

// ── internal types ────────────────────────────────────────────────────────────

/// A leaf with its full content loaded for prompt assembly.
struct LoadedLeaf {
    slug: String,
    filename: String,
    title: String,
    summary: Option<String>,
    body: String,
    collected_at: String,
}

/// Deserialized LLM response.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompileResponse {
    branches: Vec<RawBranch>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBranch {
    title: String,
    body: String,
    leaves: Vec<String>,
}

/// Deserialized incremental LLM response.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IncrementalCompileResponse {
    updated_branches: Vec<RawUpdatedBranch>,
    new_branches: Vec<RawBranch>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawUpdatedBranch {
    slug: String,
    title: String,
    body: String,
    leaves: Vec<String>,
}

/// Validated compile plan ready for execution.
#[derive(Debug, PartialEq, Eq)]
struct LeafFileClassification {
    deleted_leaf_slugs: Vec<String>,
    skipped_leaf_slugs: Vec<String>,
}

#[derive(Debug)]
struct CompilePlan {
    branches: Vec<ValidatedBranch>,
}

#[derive(Debug)]
struct ValidatedBranch {
    slug: String,
    title: String,
    body: String,
    leaves: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemovedBranchResult {
    slug: String,
    title: String,
    remaining_leaf_count: usize,
    reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchChangeKind {
    Create,
    Update,
    Rebuild,
}

#[derive(Debug)]
struct PlannedBranchWrite {
    record: BranchRecord,
    file_leaves: Vec<String>,
    body: String,
    kind: BranchChangeKind,
}

#[derive(Debug)]
struct ManifestDelta {
    new_manifest: Manifest,
    branch_writes: Vec<PlannedBranchWrite>,
    branch_deletes: Vec<String>,
    deleted_leaf_slugs: Vec<String>,
    branches_created: Vec<BranchResult>,
    branches_updated: Vec<BranchResult>,
    branches_rebuilt: Vec<BranchResult>,
    branches_removed: Vec<RemovedBranchResult>,
}

struct StagedWrite {
    pending: PendingWrite,
    bytes: Vec<u8>,
}

impl StagedWrite {
    fn new(path: String, content: String) -> Self {
        let bytes = content.into_bytes();
        Self {
            pending: PendingWrite {
                path,
                content_hash: pending::content_hash(&bytes),
            },
            bytes,
        }
    }
}

// ── cmd_compile ───────────────────────────────────────────────────────────────

pub fn cmd_compile(cfg: &SeededConfig) -> Result<(), String> {
    let result = run_compile(cfg).map_err(|e| e.to_string())?;
    print_result(&result);
    Ok(())
}

pub fn run_compile(cfg: &SeededConfig) -> Result<CompileResult, CompileError> {
    run_compile_with_options(cfg, CompileOptions::default())
}

fn preflight_noop(
    cfg: &SeededConfig,
    options: CompileOptions,
) -> Result<Option<CompileResult>, CompileError> {
    let tree = Tree::from_config(&cfg.tree);
    recover_pending_if_needed(&tree.output_dir)?;
    let manifest = manifest::read(&tree.manifest_path())
        .map_err(|e| CompileError::Io(format!("failed to read manifest: {}", e)))?;
    match manifest.leaves.len() {
        0 => return Ok(Some(CompileResult::noop("empty_tree"))),
        1 => return Ok(Some(CompileResult::noop("single_leaf"))),
        _ => {}
    }
    let new_leaf_slugs = select_new_leaf_slugs(&manifest)?;
    let leaf_file_classification = classify_leaf_files(cfg, &manifest, &new_leaf_slugs)?;
    let stale_branch_slugs =
        derive_stale_branch_slugs(&manifest, &leaf_file_classification.deleted_leaf_slugs);

    if !options.all && new_leaf_slugs.is_empty() && stale_branch_slugs.is_empty() {
        return Ok(Some(CompileResult::noop(NO_NEW_LEAVES_REASON)));
    }

    Ok(None)
}

pub fn run_compile_with_options(
    cfg: &SeededConfig,
    options: CompileOptions,
) -> Result<CompileResult, CompileError> {
    let compile_started_at = compile_timestamp_now();
    if let Some(noop) = preflight_noop(cfg, options)? {
        return Ok(noop);
    }

    let api_key = auth::resolve_openai_api_key(&auth::auth_path()).map_err(compile_auth_error)?;
    let provider = OpenAiProvider::new(api_key.api_key.as_str());
    run_compile_with_provider_started_at(
        cfg,
        options,
        &provider,
        cfg.effective_compile_model(),
        &compile_started_at,
    )
}

pub fn run_compile_with_provider(
    cfg: &SeededConfig,
    options: CompileOptions,
    provider: &dyn LlmProvider,
    model: &str,
) -> Result<CompileResult, CompileError> {
    let compile_started_at = compile_timestamp_now();
    run_compile_with_provider_started_at(cfg, options, provider, model, &compile_started_at)
}

fn run_compile_with_provider_started_at(
    cfg: &SeededConfig,
    options: CompileOptions,
    provider: &dyn LlmProvider,
    model: &str,
    compile_started_at: &str,
) -> Result<CompileResult, CompileError> {
    let tree = Tree::from_config(&cfg.tree);
    recover_pending_if_needed(&tree.output_dir)?;

    // ── read manifest (guard: empty/single-leaf) ────────────────────────────
    let manifest = manifest::read(&tree.manifest_path())
        .map_err(|e| CompileError::Io(format!("failed to read manifest: {}", e)))?;
    let expected_manifest_hash = pending::manifest_hash(&tree.output_dir)?;

    let new_leaf_slugs = select_new_leaf_slugs(&manifest)?;
    let leaf_file_classification = classify_leaf_files(cfg, &manifest, &new_leaf_slugs)?;
    let stale_branch_slugs =
        derive_stale_branch_slugs(&manifest, &leaf_file_classification.deleted_leaf_slugs);

    if !options.all && new_leaf_slugs.is_empty() && stale_branch_slugs.is_empty() {
        return Ok(CompileResult::noop(NO_NEW_LEAVES_REASON));
    }

    match manifest.leaves.len() {
        0 => return Ok(CompileResult::noop("empty_tree")),
        1 => return Ok(CompileResult::noop("single_leaf")),
        _ => {}
    }

    // ── load valid leaves ────────────────────────────────────────────────────
    let (loaded_leaves, skipped_leaves) = read_valid_leaves(cfg, &manifest.leaves);

    if loaded_leaves.is_empty() {
        return Err(CompileError::Io(format!(
            "all {} leaves have unparseable frontmatter or are missing — nothing to compile",
            skipped_leaves.len()
        )));
    }

    if loaded_leaves.len() < 2 && stale_branch_slugs.is_empty() {
        return Ok(CompileResult::noop("single_leaf"));
    }

    // ── build prompt and schema ──────────────────────────────────────────────
    let full_user_message = build_user_message(&loaded_leaves);
    let full_prompt_tokens = estimate_compile_prompt_tokens(
        COMPILE_SYSTEM_PROMPT
            .len()
            .saturating_add(full_user_message.len()),
    );
    let run_mode = if options.all {
        CompileRunMode::Full
    } else {
        CompileRunMode::Incremental
    };
    let incremental_user_message = build_incremental_user_message(
        cfg,
        &manifest,
        &loaded_leaves,
        &new_leaf_slugs,
        &stale_branch_slugs,
    );
    let incremental_prompt_tokens = estimate_compile_prompt_tokens(
        COMPILE_SYSTEM_PROMPT
            .len()
            .saturating_add(incremental_user_message.len()),
    );
    let context_mode = choose_context_mode(
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
    let schema = if run_mode == CompileRunMode::Incremental {
        incremental_compile_response_schema()
    } else {
        compile_response_schema()
    };

    // ── LLM call ─────────────────────────────────────────────────────────────
    let response = call_llm_blocking(provider, model, &user_message, &schema)?;

    // ── parse and validate ───────────────────────────────────────────────────
    let valid_filenames: HashSet<String> =
        loaded_leaves.iter().map(|l| l.filename.clone()).collect();
    let input_body_bytes = loaded_leaves.iter().map(|l| l.body.len()).sum();

    // ── execute validated plan ───────────────────────────────────────────────
    let run_timestamp = compile_started_at.to_string();
    let plan = match run_mode {
        CompileRunMode::Full => {
            parse_and_validate_with_input_size(&response, &valid_filenames, input_body_bytes)?
        }
        CompileRunMode::Incremental => parse_and_validate_incremental_with_input_size(
            &response,
            cfg,
            &valid_filenames,
            input_body_bytes,
        )?,
    };

    let summary = execute_plan_with_mode_and_expected_hash(
        &plan,
        cfg,
        &valid_filenames,
        &run_timestamp,
        &skipped_leaves,
        run_mode,
        &expected_manifest_hash,
    )?;

    Ok(CompileResult::compiled(
        summary,
        run_mode,
        context_mode,
        model,
    ))
}

// ── leaf file planning ───────────────────────────────────────────────────────

fn classify_leaf_files(
    cfg: &SeededConfig,
    manifest: &Manifest,
    new_leaf_slugs: &[String],
) -> Result<LeafFileClassification, CompileError> {
    let branch_referenced_slugs: HashSet<&str> = manifest
        .branches
        .iter()
        .flat_map(|branch| branch.leaves.iter().map(String::as_str))
        .collect();
    let new_leaf_slugs: HashSet<&str> = new_leaf_slugs.iter().map(String::as_str).collect();

    let mut deleted_leaf_slugs = Vec::new();
    let mut skipped_leaf_slugs = Vec::new();

    for leaf in &manifest.leaves {
        let leaf_path = cfg.tree.output_dir.join(&leaf.file);
        let is_new = new_leaf_slugs.contains(leaf.slug.as_str());
        let is_branch_referenced = branch_referenced_slugs.contains(leaf.slug.as_str());

        match fs::read_to_string(&leaf_path) {
            Ok(content) => {
                if frontmatter::parse(&content).is_err() {
                    if is_new {
                        return Err(CompileError::Io(format!(
                            "newly selected leaf '{}' is malformed; no files were changed",
                            leaf.file
                        )));
                    }
                    skipped_leaf_slugs.push(leaf.slug.clone());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if is_branch_referenced {
                    deleted_leaf_slugs.push(leaf.slug.clone());
                } else if is_new {
                    return Err(CompileError::Io(format!(
                        "newly selected leaf '{}' is missing; no files were changed",
                        leaf.file
                    )));
                }
            }
            Err(error) => {
                if is_new {
                    return Err(CompileError::Io(format!(
                        "newly selected leaf '{}' is unreadable: {}; no files were changed",
                        leaf.file, error
                    )));
                }
                skipped_leaf_slugs.push(leaf.slug.clone());
            }
        }
    }

    Ok(LeafFileClassification {
        deleted_leaf_slugs,
        skipped_leaf_slugs,
    })
}

// ── read_valid_leaves ─────────────────────────────────────────────────────────

fn read_valid_leaves(cfg: &SeededConfig, entries: &[LeafRecord]) -> (Vec<LoadedLeaf>, Vec<String>) {
    let mut loaded = Vec::new();
    let mut skipped = Vec::new();

    for entry in entries {
        let leaf_path = cfg.tree.output_dir.join(&entry.file);
        match fs::read_to_string(&leaf_path) {
            Ok(content) => match frontmatter::parse(&content) {
                Ok((mapping, body)) => {
                    let title = mapping
                        .get("title")
                        .and_then(|v| v.as_str())
                        .filter(|title| !title.trim().is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| entry.title.clone());
                    loaded.push(LoadedLeaf {
                        slug: entry.slug.clone(),
                        filename: entry.file.clone(),
                        title,
                        summary: entry.summary.clone(),
                        body,
                        collected_at: entry.collected_at.clone(),
                    });
                }
                Err(_) => skipped.push(entry.file.clone()),
            },
            Err(_) => skipped.push(entry.file.clone()),
        }
    }

    (loaded, skipped)
}

// ── build_user_message ────────────────────────────────────────────────────────

fn build_user_message(leaves: &[LoadedLeaf]) -> String {
    let mut msg = format!(
        "Please compile my knowledge base. There are {} documents.\n\n",
        leaves.len()
    );

    for leaf in leaves {
        msg.push_str(&format!(
            "<document filename=\"{}\" title=\"{}\">\n{}\n</document>\n\n",
            leaf.filename, leaf.title, leaf.body
        ));
    }

    msg
}

fn build_incremental_user_message(
    cfg: &SeededConfig,
    manifest: &Manifest,
    leaves: &[LoadedLeaf],
    new_leaf_slugs: &[String],
    stale_branch_slugs: &[String],
) -> String {
    let leaves_by_slug: HashMap<&str, &LoadedLeaf> = leaves
        .iter()
        .map(|leaf| (leaf.slug.as_str(), leaf))
        .collect();
    let new_leaf_slugs: HashSet<&str> = new_leaf_slugs.iter().map(String::as_str).collect();
    let stale_branch_slugs: HashSet<&str> = stale_branch_slugs.iter().map(String::as_str).collect();

    let mut msg = String::from(
        "Please incrementally compile my knowledge base. Preserve omitted non-stale branches.\n\n",
    );

    msg.push_str("<existing_branches>\n");
    for branch_record in &manifest.branches {
        let stale = stale_branch_slugs.contains(branch_record.slug.as_str());
        msg.push_str(&format!(
            "<branch slug=\"{}\" title=\"{}\" stale=\"{}\" leaves=\"{}\">\n",
            branch_record.slug,
            branch_record.title,
            stale,
            branch_record.leaves.join(",")
        ));
        let branch_path = cfg.tree.output_dir.join(&branch_record.file);
        if let Ok(content) = fs::read_to_string(branch_path) {
            if let Ok((_, body)) = frontmatter::parse(&content) {
                msg.push_str("<branch_body>\n");
                msg.push_str(&body);
                msg.push_str("\n</branch_body>\n");
            }
        }
        msg.push_str("</branch>\n");
    }
    msg.push_str("</existing_branches>\n\n");

    msg.push_str("<leaf_catalogue>\n");
    for leaf in leaves {
        msg.push_str(&format!(
            "<leaf slug=\"{}\" file=\"{}\" title=\"{}\" collected_at=\"{}\">\n",
            leaf.slug, leaf.filename, leaf.title, leaf.collected_at
        ));
        if let Some(summary) = &leaf.summary {
            msg.push_str("<summary>");
            msg.push_str(summary);
            msg.push_str("</summary>\n");
        }
        msg.push_str("</leaf>\n");
    }
    msg.push_str("</leaf_catalogue>\n\n");

    let mut full_body_slugs: HashSet<&str> = new_leaf_slugs.clone();
    for branch_record in &manifest.branches {
        if stale_branch_slugs.contains(branch_record.slug.as_str()) {
            for leaf_slug in &branch_record.leaves {
                full_body_slugs.insert(leaf_slug.as_str());
            }
        }
    }

    msg.push_str("<full_leaf_bodies>\n");
    for slug in full_body_slugs {
        if let Some(leaf) = leaves_by_slug.get(slug) {
            msg.push_str(&format!(
                "<document slug=\"{}\" filename=\"{}\" title=\"{}\">\n{}\n</document>\n",
                leaf.slug, leaf.filename, leaf.title, leaf.body
            ));
        }
    }
    msg.push_str("</full_leaf_bodies>\n");

    msg
}

// ── compile_response_schema ───────────────────────────────────────────────────

fn incremental_compile_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "updated_branches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "slug": { "type": "string" },
                        "title": { "type": "string" },
                        "body": { "type": "string" },
                        "leaves": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["slug", "title", "body", "leaves"],
                    "additionalProperties": false
                }
            },
            "new_branches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": { "type": "string" },
                        "body": { "type": "string" },
                        "leaves": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["title", "body", "leaves"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["updated_branches", "new_branches"],
        "additionalProperties": false
    })
}

fn compile_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "branches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Human-readable concept name"
                        },
                        "body": {
                            "type": "string",
                            "description": "Markdown body describing the concept across the collection"
                        },
                        "leaves": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Filenames (with .md) of leaves this concept appears in"
                        }
                    },
                    "required": ["title", "body", "leaves"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["branches"],
        "additionalProperties": false
    })
}

// ── call_llm ──────────────────────────────────────────────────────────────────

fn call_llm_blocking(
    provider: &dyn LlmProvider,
    model: &str,
    user_message: &str,
    schema: &Value,
) -> Result<String, CompileError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CompileError::Io(format!("failed to create async runtime: {}", e)))?;

    rt.block_on(call_llm_with_provider(
        provider,
        model,
        user_message,
        schema,
        COMPILE_LLM_POLICY,
    ))
}

async fn call_llm_with_provider(
    provider: &dyn LlmProvider,
    model: &str,
    user_message: &str,
    schema: &Value,
    policy: LlmCallPolicy,
) -> Result<String, CompileError> {
    let messages = vec![
        Message::system(COMPILE_SYSTEM_PROMPT),
        Message::user(user_message),
    ];

    let response = complete_with_policy(
        provider,
        &messages,
        model,
        MAX_COMPLETION_TOKENS,
        Some(schema),
        policy,
    )
    .await
    .map_err(map_compile_llm_error)?;

    match response.finish_reason {
        FinishReason::Stop => Ok(response.content),
        FinishReason::Length => Err(CompileError::Truncated),
        FinishReason::ContentFilter => Err(CompileError::ContentFilter),
        FinishReason::Other(reason) => Err(CompileError::Llm(format!(
            "unexpected finish reason: {}",
            reason
        ))),
    }
}

fn compile_auth_error(error: AuthResolutionError) -> CompileError {
    CompileError::Io(error.to_string())
}

impl From<pending::PendingError> for CompileError {
    fn from(error: pending::PendingError) -> Self {
        match error {
            pending::PendingError::Busy { .. } => CompileError::Busy(error.to_string()),
            other => CompileError::Io(other.to_string()),
        }
    }
}

fn recover_pending_if_needed(output_dir: &std::path::Path) -> Result<(), CompileError> {
    if let Some(report) = pending::recover_or_refuse(output_dir)? {
        eprintln!(
            "recovered {} changes from interrupted {}",
            report.changes, report.op
        );
    }
    Ok(())
}

fn map_compile_llm_error(error: LlmError) -> CompileError {
    let message = error.to_string();
    match error {
        LlmError::Api(msg) if msg.contains("maximum context length") => {
            CompileError::ContextOverflow {
                model: "unknown".to_string(),
                estimated_tokens: None,
                context_tokens: None,
            }
        }
        _ if message.contains("maximum context length") => CompileError::ContextOverflow {
            model: "unknown".to_string(),
            estimated_tokens: None,
            context_tokens: None,
        },
        other => CompileError::Llm(other.to_string()),
    }
}

// ── parse_and_validate ────────────────────────────────────────────────────────

fn valid_leaf_reference_map(valid_filenames: &HashSet<String>) -> HashMap<&str, String> {
    let mut refs = HashMap::new();
    for filename in valid_filenames {
        refs.insert(filename.as_str(), filename.clone());
        if let Some(stem) = filename.strip_suffix(".md") {
            refs.insert(stem, filename.clone());
        }
    }
    refs
}

fn parse_and_validate_with_input_size(
    response: &str,
    valid_filenames: &HashSet<String>,
    input_body_bytes: usize,
) -> Result<CompilePlan, CompileError> {
    let parsed: CompileResponse = serde_json::from_str(response)
        .map_err(|e| validation_error(format!("invalid compile response shape: {}", e)))?;

    // Empty branches is valid — means no cross-cutting concepts found.
    if parsed.branches.is_empty() {
        return Ok(CompilePlan {
            branches: Vec::new(),
        });
    }

    let mut validated_branches: Vec<ValidatedBranch> = Vec::new();
    let mut seen_slugs: HashSet<String> = HashSet::new();
    let valid_leaf_refs = valid_leaf_reference_map(valid_filenames);

    for (index, raw) in parsed.branches.into_iter().enumerate() {
        let branch_number = index + 1;
        let title = raw.title.trim().to_string();
        if title.is_empty() {
            return Err(validation_error(format!(
                "invalid compile response: branch #{} has empty title",
                branch_number
            )));
        }
        if raw.body.trim().is_empty() {
            return Err(validation_error(format!(
                "invalid compile response: branch '{}' has empty body",
                title
            )));
        }

        // Generate slug and check uniqueness post-slugification.
        let branch_slug = slug::slugify(&title, "");
        if branch_slug.is_empty() {
            return Err(validation_error(format!(
                "invalid compile response: branch '{}' title produces empty file slug",
                title
            )));
        }
        if seen_slugs.contains(&branch_slug) {
            return Err(validation_error(format!(
                "invalid compile response: duplicate branch slug '{}' (from title '{}') — titles must be distinct",
                branch_slug, title
            )));
        }
        seen_slugs.insert(branch_slug.clone());

        // Validate and deduplicate leaves.
        let mut branch_leaves: Vec<String> = Vec::new();
        let mut seen_leaves: HashSet<String> = HashSet::new();
        for leaf_file in &raw.leaves {
            if leaf_file.trim().is_empty() {
                return Err(validation_error(format!(
                    "invalid compile response: branch '{}' contains an empty leaf reference",
                    title
                )));
            }
            let Some(normalized_leaf_file) = valid_leaf_refs.get(leaf_file.as_str()) else {
                return Err(validation_error(format!(
                    "invalid compile response: branch '{}' references unknown leaf '{}'",
                    title, leaf_file
                )));
            };
            if seen_leaves.insert(normalized_leaf_file.clone()) {
                branch_leaves.push(normalized_leaf_file.clone());
            }
        }

        if branch_leaves.len() < 2 {
            return Err(validation_error(format!(
                "invalid compile response: branch '{}' references {} leaf; branches must reference at least 2 leaves",
                title,
                branch_leaves.len()
            )));
        }

        validated_branches.push(ValidatedBranch {
            slug: branch_slug,
            title,
            body: raw.body,
            leaves: branch_leaves,
        });
    }

    let output_body_bytes = validated_branches
        .iter()
        .map(|branch| branch.body.len())
        .fold(0usize, usize::saturating_add);
    validate_compiled_body_size(input_body_bytes, output_body_bytes)?;

    Ok(CompilePlan {
        branches: validated_branches,
    })
}

fn parse_and_validate_incremental_with_input_size(
    response: &str,
    cfg: &SeededConfig,
    valid_filenames: &HashSet<String>,
    input_body_bytes: usize,
) -> Result<CompilePlan, CompileError> {
    let parsed: IncrementalCompileResponse = serde_json::from_str(response).map_err(|e| {
        validation_error(format!("invalid incremental compile response shape: {}", e))
    })?;
    let tree = Tree::from_config(&cfg.tree);
    let manifest = manifest::read(&tree.manifest_path())
        .map_err(|e| CompileError::Io(format!("failed to read manifest: {}", e)))?;
    let new_leaf_slugs_vec = select_new_leaf_slugs(&manifest)?;
    let classification = classify_leaf_files(cfg, &manifest, &new_leaf_slugs_vec)?;
    let stale_branch_slugs: HashSet<String> =
        derive_stale_branch_slugs(&manifest, &classification.deleted_leaf_slugs)
            .into_iter()
            .collect();
    let deleted_leaf_slugs: HashSet<String> =
        classification.deleted_leaf_slugs.into_iter().collect();
    let new_leaf_slugs: HashSet<String> = new_leaf_slugs_vec.into_iter().collect();
    let valid_leaf_refs = valid_leaf_reference_map(valid_filenames);
    let required_stale_rebuild_slugs: HashSet<String> = manifest
        .branches
        .iter()
        .filter(|branch| stale_branch_slugs.contains(&branch.slug))
        .filter(|branch| {
            branch
                .leaves
                .iter()
                .filter(|leaf| !deleted_leaf_slugs.contains(*leaf))
                .count()
                >= 2
        })
        .map(|branch| branch.slug.clone())
        .collect();
    let mut seen_branch_slugs = HashSet::new();
    let mut seen_updated_branch_slugs = HashSet::new();
    let mut validated_branches = Vec::new();

    for raw in parsed.updated_branches {
        let existing = manifest.branch_by_slug(&raw.slug).ok_or_else(|| {
            validation_error(format!(
                "invalid incremental compile response: update references unknown branch '{}'",
                raw.slug
            ))
        })?;
        if raw.title.trim() != existing.title {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' changed title",
                raw.slug
            )));
        }
        let leaves = normalize_incremental_leaf_refs(&raw.title, &raw.leaves, &valid_leaf_refs)?;
        let leaf_slugs: HashSet<String> = leaves
            .iter()
            .map(|leaf| leaf.strip_suffix(".md").unwrap_or(leaf).to_string())
            .collect();
        let is_stale = stale_branch_slugs.contains(&raw.slug);
        let remaining_valid_existing_leaves: HashSet<String> = existing
            .leaves
            .iter()
            .filter(|leaf| !deleted_leaf_slugs.contains(*leaf))
            .cloned()
            .collect();
        if is_stale {
            if remaining_valid_existing_leaves.len() < 2 {
                return Err(validation_error(format!(
                    "invalid incremental compile response: stale branch '{}' has fewer than 2 remaining valid leaves and must be removed deterministically",
                    raw.slug
                )));
            }
            for leaf_slug in &leaf_slugs {
                if !remaining_valid_existing_leaves.contains(leaf_slug) {
                    return Err(validation_error(format!(
                        "invalid incremental compile response: stale branch '{}' may only use remaining valid leaves",
                        raw.slug
                    )));
                }
            }
        }
        for existing_leaf in &existing.leaves {
            if deleted_leaf_slugs.contains(existing_leaf) {
                continue;
            }
            if !leaf_slugs.contains(existing_leaf) {
                return Err(validation_error(format!(
                    "invalid incremental compile response: branch '{}' dropped existing leaf '{}'",
                    raw.slug, existing_leaf
                )));
            }
        }
        if !is_stale && !leaf_slugs.iter().any(|slug| new_leaf_slugs.contains(slug)) {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' update adds no newly processed leaf",
                raw.slug
            )));
        }
        if leaves.len() < 2 {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' references {} leaf; branches must reference at least 2 leaves",
                raw.title,
                leaves.len()
            )));
        }
        if !seen_updated_branch_slugs.insert(raw.slug.clone())
            || !seen_branch_slugs.insert(raw.slug.clone())
        {
            return Err(validation_error(format!(
                "invalid incremental compile response: duplicate branch slug '{}'",
                raw.slug
            )));
        }
        validated_branches.push(ValidatedBranch {
            slug: raw.slug,
            title: raw.title,
            body: raw.body,
            leaves,
        });
    }

    for stale_slug in &required_stale_rebuild_slugs {
        if !seen_updated_branch_slugs.contains(stale_slug) {
            return Err(validation_error(format!(
                "invalid incremental compile response: stale branch '{}' must be rebuilt or removed deterministically",
                stale_slug
            )));
        }
    }

    for raw in parsed.new_branches {
        let title = raw.title.trim().to_string();
        if title.is_empty() {
            return Err(validation_error(
                "invalid incremental compile response: new branch has empty title",
            ));
        }
        if raw.body.trim().is_empty() {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' has empty body",
                title
            )));
        }
        let branch_slug = slug::slugify(&title, "");
        if manifest.branch_by_slug(&branch_slug).is_some()
            || !seen_branch_slugs.insert(branch_slug.clone())
        {
            return Err(validation_error(format!(
                "invalid incremental compile response: duplicate branch slug '{}'",
                branch_slug
            )));
        }
        let leaves = normalize_incremental_leaf_refs(&title, &raw.leaves, &valid_leaf_refs)?;
        if leaves.len() < 2 {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' references {} leaf; branches must reference at least 2 leaves",
                title,
                leaves.len()
            )));
        }
        if !leaves.iter().any(|leaf| {
            let slug = leaf.strip_suffix(".md").unwrap_or(leaf);
            new_leaf_slugs.contains(slug)
        }) {
            return Err(validation_error(format!(
                "invalid incremental compile response: new branch '{}' contains no newly processed leaf",
                title
            )));
        }
        validated_branches.push(ValidatedBranch {
            slug: branch_slug,
            title,
            body: raw.body,
            leaves,
        });
    }

    let output_body_bytes = validated_branches
        .iter()
        .map(|branch| branch.body.len())
        .fold(0usize, usize::saturating_add);
    validate_compiled_body_size(input_body_bytes, output_body_bytes)?;

    Ok(CompilePlan {
        branches: validated_branches,
    })
}

fn normalize_incremental_leaf_refs(
    branch_title: &str,
    raw_leaves: &[String],
    valid_leaf_refs: &HashMap<&str, String>,
) -> Result<Vec<String>, CompileError> {
    let mut leaves = Vec::new();
    let mut seen = HashSet::new();
    for raw_leaf in raw_leaves {
        let Some(normalized) = valid_leaf_refs.get(raw_leaf.as_str()) else {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' references unknown leaf '{}'",
                branch_title, raw_leaf
            )));
        };
        if seen.insert(normalized.clone()) {
            leaves.push(normalized.clone());
        }
    }
    Ok(leaves)
}

fn validate_compiled_body_size(
    input_body_bytes: usize,
    output_body_bytes: usize,
) -> Result<(), CompileError> {
    let limit = input_body_bytes
        .saturating_mul(MAX_COMPILED_BODY_BYTES_PER_INPUT_BYTE)
        .max(MAX_COMPILED_BODY_BYTES_MIN);

    if output_body_bytes > limit {
        return Err(validation_error(format!(
            "invalid compile response: branch bodies total {} bytes, exceeding {} byte limit for {} bytes of input",
            output_body_bytes, limit, input_body_bytes
        )));
    }

    Ok(())
}

fn validation_error(message: impl Into<String>) -> CompileError {
    CompileError::Validation(message.into())
}

fn branch_result(slug: &str, title: &str, leaf_count: usize) -> BranchResult {
    BranchResult {
        slug: slug.to_string(),
        title: title.to_string(),
        leaf_count,
    }
}

fn validated_branch_leaf_slugs(branch: &ValidatedBranch) -> Vec<String> {
    branch
        .leaves
        .iter()
        .map(|leaf| leaf.strip_suffix(".md").unwrap_or(leaf).to_string())
        .collect()
}

fn build_manifest_delta(
    current: &Manifest,
    plan: &CompilePlan,
    run_mode: CompileRunMode,
    run_timestamp: &str,
    deleted_leaf_slugs: &[String],
    stale_branch_slugs: &[String],
) -> Result<ManifestDelta, CompileError> {
    let deleted_leaf_slugs_set: HashSet<&str> =
        deleted_leaf_slugs.iter().map(String::as_str).collect();
    let stale_branch_slugs_set: HashSet<&str> =
        stale_branch_slugs.iter().map(String::as_str).collect();

    let mut branch_writes = Vec::new();
    let mut branch_deletes = Vec::new();
    let mut branches_created = Vec::new();
    let mut branches_updated = Vec::new();
    let mut branches_rebuilt = Vec::new();
    let mut branches_removed = Vec::new();
    let mut new_branches = Vec::new();

    match run_mode {
        CompileRunMode::Full => {
            let planned_slugs: HashSet<&str> = plan
                .branches
                .iter()
                .map(|branch| branch.slug.as_str())
                .collect();
            for branch in &current.branches {
                if !planned_slugs.contains(branch.slug.as_str()) {
                    branch_deletes.push(branch.file.clone());
                }
            }
            for planned in &plan.branches {
                let created_at = current
                    .branch_by_slug(&planned.slug)
                    .map(|branch| branch.created_at.clone())
                    .unwrap_or_else(|| run_timestamp.to_string());
                let record = BranchRecord {
                    slug: planned.slug.clone(),
                    file: format!("branches/{}.md", planned.slug),
                    title: planned.title.clone(),
                    created_at,
                    updated_at: run_timestamp.to_string(),
                    stale: false,
                    leaves: validated_branch_leaf_slugs(planned),
                };
                if current.branch_by_slug(&planned.slug).is_some() {
                    branches_updated.push(branch_result(
                        &record.slug,
                        &record.title,
                        record.leaves.len(),
                    ));
                    branch_writes.push(PlannedBranchWrite {
                        record: record.clone(),
                        file_leaves: planned.leaves.clone(),
                        body: planned.body.clone(),
                        kind: BranchChangeKind::Update,
                    });
                } else {
                    branches_created.push(branch_result(
                        &record.slug,
                        &record.title,
                        record.leaves.len(),
                    ));
                    branch_writes.push(PlannedBranchWrite {
                        record: record.clone(),
                        file_leaves: planned.leaves.clone(),
                        body: planned.body.clone(),
                        kind: BranchChangeKind::Create,
                    });
                }
                new_branches.push(record);
            }
        }
        CompileRunMode::Incremental => {
            let planned_by_slug: HashMap<&str, &ValidatedBranch> = plan
                .branches
                .iter()
                .map(|branch| (branch.slug.as_str(), branch))
                .collect();
            let current_branch_slugs: HashSet<&str> = current
                .branches
                .iter()
                .map(|branch| branch.slug.as_str())
                .collect();
            for current_branch in &current.branches {
                let is_stale = stale_branch_slugs_set.contains(current_branch.slug.as_str());
                let remaining_leaf_slugs: Vec<String> = current_branch
                    .leaves
                    .iter()
                    .filter(|leaf| !deleted_leaf_slugs_set.contains(leaf.as_str()))
                    .cloned()
                    .collect();
                if is_stale && remaining_leaf_slugs.len() < 2 {
                    branch_deletes.push(current_branch.file.clone());
                    branches_removed.push(RemovedBranchResult {
                        slug: current_branch.slug.clone(),
                        title: current_branch.title.clone(),
                        remaining_leaf_count: remaining_leaf_slugs.len(),
                        reason: "stale_branch_below_minimum_leaves".to_string(),
                    });
                    continue;
                }
                if let Some(planned) = planned_by_slug.get(current_branch.slug.as_str()) {
                    let record = BranchRecord {
                        slug: current_branch.slug.clone(),
                        file: current_branch.file.clone(),
                        title: planned.title.clone(),
                        created_at: current_branch.created_at.clone(),
                        updated_at: run_timestamp.to_string(),
                        stale: false,
                        leaves: validated_branch_leaf_slugs(planned),
                    };
                    let result = branch_result(&record.slug, &record.title, record.leaves.len());
                    if is_stale {
                        branches_rebuilt.push(result.clone());
                        branch_writes.push(PlannedBranchWrite {
                            record: record.clone(),
                            file_leaves: planned.leaves.clone(),
                            body: planned.body.clone(),
                            kind: BranchChangeKind::Rebuild,
                        });
                    } else {
                        branches_updated.push(result.clone());
                        branch_writes.push(PlannedBranchWrite {
                            record: record.clone(),
                            file_leaves: planned.leaves.clone(),
                            body: planned.body.clone(),
                            kind: BranchChangeKind::Update,
                        });
                    }
                    new_branches.push(record);
                } else if is_stale {
                    return Err(validation_error(format!(
                        "invalid incremental compile response: stale branch '{}' must be rebuilt or removed deterministically",
                        current_branch.slug
                    )));
                } else {
                    new_branches.push(current_branch.clone());
                }
            }
            for planned in &plan.branches {
                if current_branch_slugs.contains(planned.slug.as_str()) {
                    continue;
                }
                let record = BranchRecord {
                    slug: planned.slug.clone(),
                    file: format!("branches/{}.md", planned.slug),
                    title: planned.title.clone(),
                    created_at: run_timestamp.to_string(),
                    updated_at: run_timestamp.to_string(),
                    stale: false,
                    leaves: validated_branch_leaf_slugs(planned),
                };
                branches_created.push(branch_result(
                    &record.slug,
                    &record.title,
                    record.leaves.len(),
                ));
                branch_writes.push(PlannedBranchWrite {
                    record: record.clone(),
                    file_leaves: planned.leaves.clone(),
                    body: planned.body.clone(),
                    kind: BranchChangeKind::Create,
                });
                new_branches.push(record);
            }
        }
    }

    let new_manifest = Manifest {
        tree: TreeMeta {
            name: current.tree.name.clone(),
            created_at: current.tree.created_at.clone(),
            last_compiled_at: Some(run_timestamp.to_string()),
        },
        leaves: current
            .leaves
            .iter()
            .filter(|leaf| !deleted_leaf_slugs_set.contains(leaf.slug.as_str()))
            .cloned()
            .collect(),
        branches: new_branches,
    };

    Ok(ManifestDelta {
        new_manifest,
        branch_writes,
        branch_deletes,
        deleted_leaf_slugs: deleted_leaf_slugs.to_vec(),
        branches_created,
        branches_updated,
        branches_rebuilt,
        branches_removed,
    })
}

// ── execute_plan ──────────────────────────────────────────────────────────────

fn execute_plan_with_mode_and_expected_hash(
    plan: &CompilePlan,
    cfg: &SeededConfig,
    valid_filenames: &HashSet<String>,
    run_timestamp: &str,
    skipped_leaves: &[String],
    run_mode: CompileRunMode,
    expected_manifest_hash: &str,
) -> Result<CompileSummary, CompileError> {
    let tree = Tree::from_config(&cfg.tree);
    recover_pending_if_needed(&tree.output_dir)?;

    // Load current manifest. Used to preserve branch `created_at` and carry
    // leaf records / tree metadata forward into the new manifest.
    let current = match manifest::read(&tree.manifest_path()) {
        Ok(m) => m,
        Err(manifest::ManifestError::TreeNotInitialized) => Manifest {
            tree: TreeMeta {
                name: tree.name.clone().unwrap_or_else(|| "unnamed".to_string()),
                created_at: tree
                    .created_at
                    .clone()
                    .unwrap_or_else(|| run_timestamp.to_string()),
                last_compiled_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        },
        Err(e) => return Err(CompileError::Io(format!("failed to read manifest: {}", e))),
    };

    let current_manifest_hash = pending::manifest_hash(&tree.output_dir)?;
    if current_manifest_hash != expected_manifest_hash {
        return Err(CompileError::Io(
            "manifest changed during compile planning; rerun `bo compile`".to_string(),
        ));
    }

    let new_leaf_slugs = select_new_leaf_slugs(&current)?;
    let classification = classify_leaf_files(cfg, &current, &new_leaf_slugs)?;
    let stale_branch_slugs =
        derive_stale_branch_slugs(&current, &classification.deleted_leaf_slugs);
    let delta = build_manifest_delta(
        &current,
        plan,
        run_mode,
        run_timestamp,
        &classification.deleted_leaf_slugs,
        &stale_branch_slugs,
    )?;

    let mut staged: Vec<StagedWrite> = Vec::new();
    for planned_write in &delta.branch_writes {
        let _change_kind = planned_write.kind;
        let content = branch::format_content(
            &planned_write.record.title,
            &planned_write.body,
            &planned_write.file_leaves,
            &planned_write.record.created_at,
            run_timestamp,
        );
        staged.push(StagedWrite::new(planned_write.record.file.clone(), content));
    }

    let leaves_processed = valid_filenames.len();

    let writes: Vec<PendingWrite> = staged.iter().map(|write| write.pending.clone()).collect();
    let compile_mode = match run_mode {
        CompileRunMode::Incremental => CompileMode::Incremental,
        CompileRunMode::Full => CompileMode::Full,
    };
    let operation = pending::new_operation(
        &tree.output_dir,
        OpKind::Compile { mode: compile_mode },
        writes.clone(),
        delta.branch_deletes.clone(),
    )?;
    let pending_path = pending::pending_path(&tree.output_dir);
    pending::write(&pending_path, &operation)?;
    for write in &staged {
        pending::write_staged(&tree.output_dir, &write.pending, &write.bytes)?;
    }
    manifest::write(&tree.manifest_path(), &delta.new_manifest)
        .map_err(|e| CompileError::Io(format!("failed to write manifest: {}", e)))?;
    pending::apply_writes(&tree.output_dir, &writes)?;
    pending::apply_deletes(&tree.output_dir, &delta.branch_deletes)?;
    pending::clear(&pending_path)?;

    let _deleted_leaf_slugs = &delta.deleted_leaf_slugs;
    let _removed_branches = &delta.branches_removed;
    let branch_results: Vec<BranchResult> = delta
        .branches_created
        .iter()
        .chain(delta.branches_updated.iter())
        .chain(delta.branches_rebuilt.iter())
        .cloned()
        .collect();

    for branch in &branch_results {
        eprintln!("writing branch: {}", branch.slug);
    }

    Ok(CompileSummary {
        branches: branch_results,
        leaves_processed,
        leaves_skipped: skipped_leaves.to_vec(),
    })
}

// ── print_summary ─────────────────────────────────────────────────────────────

pub fn print_result(result: &CompileResult) {
    if result.status == "noop" {
        match result.reason.as_deref() {
            Some("empty_tree") => println!("bo is empty!"),
            Some("single_leaf") => println!("bo only has 1 leaf!"),
            Some(NO_NEW_LEAVES_REASON) => println!("nothing new to compile"),
            _ => println!("compiled: no work to do"),
        }
        return;
    }

    print_summary_parts(
        &result.branches,
        result.leaves_processed,
        &result.leaves_skipped,
    );
}

fn print_summary_parts(
    branches: &[BranchResult],
    leaves_processed: usize,
    leaves_skipped: &[String],
) {
    if branches.is_empty() {
        println!("compiled: no branches found");
    } else {
        println!(
            "compiled: {} {} from {} processed leaves",
            branches.len(),
            if branches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            leaves_processed
        );
        for b in branches {
            println!(
                "  ✓ {} ({} {})",
                b.slug,
                b.leaf_count,
                if b.leaf_count == 1 { "leaf" } else { "leaves" }
            );
        }
    }

    if !leaves_skipped.is_empty() {
        println!();
        println!(
            "  ⚠ skipped {} {} (unparseable frontmatter):",
            leaves_skipped.len(),
            if leaves_skipped.len() == 1 {
                "leaf"
            } else {
                "leaves"
            }
        );
        for f in leaves_skipped {
            println!("    - {}", f);
        }
    }
}

#[cfg(test)]
#[path = "../../tests/cli_compile_tests.rs"]
mod tests;
