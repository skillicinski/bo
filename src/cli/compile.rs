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

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::domain::{branch, frontmatter, index, slug, tree, tree::Tree};
use crate::engine::auth::{self, AuthResolutionError};
use crate::engine::config::SeededConfig;
use crate::engine::llm::{
    complete_with_policy, FinishReason, LlmCallPolicy, LlmError, LlmProvider, Message,
    OpenAiProvider,
};
use crate::engine::state;

// ── constants ─────────────────────────────────────────────────────────────────

const MAX_COMPLETION_TOKENS: u32 = 16384;
const MAX_COMPILED_BODY_BYTES_MIN: usize = 16 * 1024;
const MAX_COMPILED_BODY_BYTES_PER_INPUT_BYTE: usize = 8;

pub const VALIDATION_NEXT_STEP: &str = "No files were changed. Try `bo compile` again; if this repeats, switch models with `bo config set model <model>` or report the validation message.";

const COMPILE_LLM_POLICY: LlmCallPolicy = LlmCallPolicy {
    timeout: Duration::from_secs(180),
    max_attempts: 3,
    initial_backoff: Duration::from_secs(2),
};

// ── errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CompileError {
    /// Collection exceeds the model's context window.
    ContextOverflow,
    /// LLM output was truncated (hit max_completion_tokens).
    Truncated,
    /// Response blocked by content filter.
    ContentFilter,
    /// LLM API or network error.
    Llm(String),
    /// I/O or index error.
    Io(String),
    /// Validation error in the LLM response.
    Validation(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::ContextOverflow => write!(
                f,
                "collection too large for model context window — \
                 try reducing collection size or using a model with larger context"
            ),
            CompileError::Truncated => write!(
                f,
                "compile output was truncated — try reducing collection size or \
                 using a model with larger output capacity"
            ),
            CompileError::ContentFilter => write!(f, "compile was blocked by content filter"),
            CompileError::Llm(msg) => write!(f, "LLM error: {}", msg),
            CompileError::Io(msg) => write!(f, "{}", msg),
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

#[derive(Debug, Clone, Serialize)]
pub struct CompileResult {
    pub status: String,
    pub reason: Option<String>,
    pub branches: Vec<BranchResult>,
    pub leaves_updated: usize,
    pub leaves_skipped: Vec<String>,
}

impl CompileResult {
    fn compiled(summary: CompileSummary) -> Self {
        Self {
            status: "compiled".to_string(),
            reason: None,
            branches: summary.branches,
            leaves_updated: summary.leaves_updated,
            leaves_skipped: summary.leaves_skipped,
        }
    }

    fn noop(reason: &str) -> Self {
        Self {
            status: "noop".to_string(),
            reason: Some(reason.to_string()),
            branches: Vec::new(),
            leaves_updated: 0,
            leaves_skipped: Vec::new(),
        }
    }
}

pub struct CompileSummary {
    pub branches: Vec<BranchResult>,
    pub leaves_updated: usize,
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
    filename: String,
    title: String,
    body: String,
}

/// Deserialized LLM response.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CompileResponse {
    branches: Vec<RawBranch>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawBranch {
    title: String,
    body: String,
    leaves: Vec<String>,
}

/// Validated compile plan ready for execution.
#[derive(Debug)]
struct CompilePlan {
    branches: Vec<ValidatedBranch>,
    /// leaf filename → list of branch slugs it belongs to
    leaf_assignments: HashMap<String, Vec<String>>,
}

#[derive(Debug)]
struct ValidatedBranch {
    slug: String,
    title: String,
    body: String,
    leaves: Vec<String>,
}

// ── cmd_compile ───────────────────────────────────────────────────────────────

pub fn cmd_compile(cfg: &SeededConfig) -> Result<(), String> {
    let result = run_compile(cfg).map_err(|e| e.to_string())?;
    print_result(&result);
    Ok(())
}

pub fn run_compile(cfg: &SeededConfig) -> Result<CompileResult, CompileError> {
    // ── read index (guard: empty/single-leaf) ────────────────────────────────
    let index_path = tree::index_path(&cfg.tree.output_dir);
    let all_entries = index::read_index(&index_path)
        .map_err(|e| CompileError::Io(format!("failed to read index: {}", e)))?;

    match all_entries.len() {
        0 => return Ok(CompileResult::noop("empty_tree")),
        1 => return Ok(CompileResult::noop("single_leaf")),
        _ => {}
    }

    // ── resolve OpenAI auth ──────────────────────────────────────────────────
    let api_key = auth::resolve_openai_api_key(&auth::auth_path()).map_err(compile_auth_error)?;

    // ── load valid leaves ────────────────────────────────────────────────────
    let (loaded_leaves, skipped_leaves) = read_valid_leaves(cfg, &all_entries);

    if loaded_leaves.is_empty() {
        return Err(CompileError::Io(format!(
            "all {} leaves have unparseable frontmatter or are missing — nothing to compile",
            skipped_leaves.len()
        )));
    }

    if loaded_leaves.len() < 2 {
        return Ok(CompileResult::noop("single_leaf"));
    }

    // ── build prompt and schema ──────────────────────────────────────────────
    let user_message = build_user_message(&loaded_leaves);
    let schema = compile_response_schema();

    // ── LLM call ─────────────────────────────────────────────────────────────
    let response = call_llm(
        api_key.api_key.as_str(),
        cfg.effective_model(),
        &user_message,
        &schema,
    )?;

    // ── parse and validate ───────────────────────────────────────────────────
    let valid_filenames: HashSet<String> =
        loaded_leaves.iter().map(|l| l.filename.clone()).collect();
    let input_body_bytes = loaded_leaves.iter().map(|l| l.body.len()).sum();

    // ── execute validated plan ───────────────────────────────────────────────
    let run_timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let summary = validate_and_execute_plan(
        &response,
        cfg,
        &valid_filenames,
        input_body_bytes,
        &run_timestamp,
        &skipped_leaves,
    )?;

    // ── output ───────────────────────────────────────────────────────────────

    // Persist compiled leaf slugs to state.json
    let state_path = tree::state_path(&cfg.tree.output_dir);
    let mut tree_state = state::read_state(&state_path);
    for filename in &valid_filenames {
        let slug = state::slug_from_filename(filename);
        tree_state
            .compiled_leaves
            .insert(slug.to_string(), run_timestamp.clone());
    }
    if let Err(e) = state::write_state(&state_path, &tree_state) {
        eprintln!("warning: failed to write compile state: {}", e);
    }

    Ok(CompileResult::compiled(summary))
}

// ── read_valid_leaves ─────────────────────────────────────────────────────────

fn read_valid_leaves(
    cfg: &SeededConfig,
    entries: &[index::IndexEntry],
) -> (Vec<LoadedLeaf>, Vec<String>) {
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
                        .unwrap_or("")
                        .to_string();
                    loaded.push(LoadedLeaf {
                        filename: entry.file.clone(),
                        title,
                        body,
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

// ── compile_response_schema ───────────────────────────────────────────────────

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

fn call_llm(
    api_key: &str,
    model: &str,
    user_message: &str,
    schema: &Value,
) -> Result<String, CompileError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CompileError::Io(format!("failed to create async runtime: {}", e)))?;

    let provider = OpenAiProvider::new(api_key);
    rt.block_on(call_llm_with_provider(
        &provider,
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

fn map_compile_llm_error(error: LlmError) -> CompileError {
    let message = error.to_string();
    match error {
        LlmError::Api(msg) if msg.contains("maximum context length") => {
            CompileError::ContextOverflow
        }
        _ if message.contains("maximum context length") => CompileError::ContextOverflow,
        other => CompileError::Llm(other.to_string()),
    }
}

// ── parse_and_validate ────────────────────────────────────────────────────────

#[cfg(test)]
fn parse_and_validate(
    response: &str,
    valid_filenames: &HashSet<String>,
) -> Result<CompilePlan, CompileError> {
    parse_and_validate_with_input_size(response, valid_filenames, usize::MAX)
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
            leaf_assignments: HashMap::new(),
        });
    }

    let mut validated_branches: Vec<ValidatedBranch> = Vec::new();
    let mut seen_slugs: HashSet<String> = HashSet::new();
    let mut leaf_assignments: HashMap<String, Vec<String>> = HashMap::new();

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
            if !valid_filenames.contains(leaf_file) {
                return Err(validation_error(format!(
                    "invalid compile response: branch '{}' references unknown leaf '{}'",
                    title, leaf_file
                )));
            }
            if seen_leaves.insert(leaf_file.clone()) {
                branch_leaves.push(leaf_file.clone());
            }
        }

        if branch_leaves.len() < 2 {
            return Err(validation_error(format!(
                "invalid compile response: branch '{}' references {} leaf; branches must reference at least 2 leaves",
                title,
                branch_leaves.len()
            )));
        }

        // Record leaf assignments.
        for leaf_file in &branch_leaves {
            leaf_assignments
                .entry(leaf_file.clone())
                .or_default()
                .push(branch_slug.clone());
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
        leaf_assignments,
    })
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

fn validate_and_execute_plan(
    response: &str,
    cfg: &SeededConfig,
    valid_filenames: &HashSet<String>,
    input_body_bytes: usize,
    run_timestamp: &str,
    skipped_leaves: &[String],
) -> Result<CompileSummary, CompileError> {
    let plan = parse_and_validate_with_input_size(response, valid_filenames, input_body_bytes)?;
    execute_plan(&plan, cfg, valid_filenames, run_timestamp, skipped_leaves)
}

// ── execute_plan ──────────────────────────────────────────────────────────────

fn execute_plan(
    plan: &CompilePlan,
    cfg: &SeededConfig,
    valid_filenames: &HashSet<String>,
    run_timestamp: &str,
    skipped_leaves: &[String],
) -> Result<CompileSummary, CompileError> {
    let tree = Tree::from_config(&cfg.tree);
    let branches_dir = tree.branches_dir();

    // ── write branches ───────────────────────────────────────────────────────
    let mut branch_results: Vec<BranchResult> = Vec::new();

    for vb in &plan.branches {
        let branch_path = branches_dir.join(format!("{}.md", vb.slug));

        // Preserve compiled_at from existing branch if it exists
        let compiled_at =
            branch::read_compiled_at(&branch_path).unwrap_or_else(|| run_timestamp.to_string());

        branch::write(
            &branch_path,
            &vb.title,
            &vb.body,
            &vb.leaves,
            &compiled_at,
            run_timestamp,
        )
        .map_err(|e| CompileError::Io(format!("failed to write branch '{}': {}", vb.slug, e)))?;

        eprintln!("writing branch: {}", vb.slug);

        branch_results.push(BranchResult {
            slug: vb.slug.clone(),
            title: vb.title.clone(),
            leaf_count: vb.leaves.len(),
        });
    }

    // ── update leaf frontmatter ──────────────────────────────────────────────
    let mut leaves_updated: usize = 0;

    for filename in valid_filenames {
        let leaf_path = cfg.tree.output_dir.join(filename);

        // Get this leaf's branch assignments (empty if not in any branch)
        let branches: Vec<String> = plan
            .leaf_assignments
            .get(filename)
            .cloned()
            .unwrap_or_default();

        let content = match fs::read_to_string(&leaf_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("warning: could not read '{}': {} — skipping", filename, e);
                continue;
            }
        };

        let updated = match frontmatter::patch_fields(
            &content,
            &[("updated_at", run_timestamp)],
            &[("branches", &branches)],
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "warning: could not patch frontmatter of '{}': {} — skipping",
                    filename, e
                );
                continue;
            }
        };

        if let Err(e) = fs::write(&leaf_path, &updated) {
            eprintln!("warning: could not write '{}': {} — skipping", filename, e);
            continue;
        }

        leaves_updated += 1;
    }

    Ok(CompileSummary {
        branches: branch_results,
        leaves_updated,
        leaves_skipped: skipped_leaves.to_vec(),
    })
}

// ── print_summary ─────────────────────────────────────────────────────────────

pub fn print_result(result: &CompileResult) {
    if result.status == "noop" {
        match result.reason.as_deref() {
            Some("empty_tree") => println!("bo is empty!"),
            Some("single_leaf") => println!("bo only has 1 leaf!"),
            _ => println!("compiled: no work to do"),
        }
        return;
    }

    print_summary_parts(
        &result.branches,
        result.leaves_updated,
        &result.leaves_skipped,
    );
}

fn print_summary_parts(
    branches: &[BranchResult],
    leaves_updated: usize,
    leaves_skipped: &[String],
) {
    if branches.is_empty() {
        println!("compiled: no branches found");
    } else {
        println!(
            "compiled: {} {} across {} leaves",
            branches.len(),
            if branches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            leaves_updated
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
#[path = "../tests/cli_compile_tests.rs"]
mod tests;
