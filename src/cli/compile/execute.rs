// ── compile execution: LLM calls, staging, disk writes ────────────────────────

use std::collections::HashSet;

use serde_json::Value;

use crate::domain::{branch, manifest, Timestamp};

use crate::engine::config::SeededConfig;
use crate::engine::llm::{
    complete_with_policy, FinishReason, LlmCallPolicy, LlmError, LlmProvider, Message, Model,
};
use crate::engine::pending::{self, CompileMode, OpKind, PendingWrite};

use super::parse::CompilePlan;
use super::plan::build_manifest_delta;
use super::prompt::COMPILE_SYSTEM_PROMPT;
use super::{
    BranchResult, CompileContextMode, CompileError, CompileRunMode, CompileSummary,
    COMPILE_LLM_POLICY, COMPILE_PROMPT_OVERHEAD_TOKENS, MAX_COMPLETION_TOKENS,
    TOKEN_ESTIMATE_BYTES_PER_TOKEN,
};

// ── types ─────────────────────────────────────────────────────────────────────

pub(super) struct StagedWrite {
    pub(super) pending: PendingWrite,
    pub(super) bytes: Vec<u8>,
}

impl StagedWrite {
    pub(super) fn new(path: String, content: String) -> Self {
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

// ── time helpers ──────────────────────────────────────────────────────────────

pub(super) fn compile_timestamp_now() -> Timestamp {
    Timestamp::now()
}

pub(super) fn collected_after_last_compile(
    collected_at: &Timestamp,
    last_compiled_at: &Timestamp,
) -> bool {
    collected_at > last_compiled_at
}

// ── token estimation ──────────────────────────────────────────────────────────

pub(super) fn estimate_tokens_from_bytes(bytes: usize) -> usize {
    bytes.div_ceil(TOKEN_ESTIMATE_BYTES_PER_TOKEN)
}

pub(super) fn estimate_compile_prompt_tokens(prompt_bytes: usize) -> usize {
    COMPILE_PROMPT_OVERHEAD_TOKENS
        .saturating_add(MAX_COMPLETION_TOKENS as usize)
        .saturating_add(estimate_tokens_from_bytes(prompt_bytes))
}

pub(super) fn ensure_compile_context_fits(
    model: &Model,
    estimated_tokens: usize,
) -> Result<(), CompileError> {
    let context_tokens = model.context_tokens();

    if estimated_tokens > context_tokens {
        return Err(CompileError::ContextOverflow {
            model: model.to_string(),
            estimated_tokens: Some(estimated_tokens),
            context_tokens: Some(context_tokens),
        });
    }

    Ok(())
}

pub(super) fn choose_context_mode(
    model: &Model,
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

// ── LLM call ──────────────────────────────────────────────────────────────────

pub(super) fn call_llm_blocking(
    provider: &dyn LlmProvider,
    model: &Model,
    user_message: &str,
    schema: &Value,
) -> Result<String, CompileError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CompileError::Io(format!("failed to create async runtime: {}", e)))?;

    rt.block_on(call_llm_with_provider(
        provider,
        model.as_str(),
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
        false,
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

pub(super) fn map_compile_llm_error(error: LlmError) -> CompileError {
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

// ── pending/recovery ──────────────────────────────────────────────────────────

pub(super) fn recover_pending_if_needed(path: &std::path::Path) -> Result<(), CompileError> {
    if let Some(report) = pending::recover_or_refuse(path)? {
        eprintln!(
            "recovered {} changes from interrupted {}",
            report.changes, report.op
        );
    }
    Ok(())
}

// ── execute plan ──────────────────────────────────────────────────────────────

pub(super) fn execute_plan_with_mode_and_expected_hash(
    plan: &CompilePlan,
    cfg: &SeededConfig,
    valid_filenames: &HashSet<String>,
    run_timestamp: &Timestamp,
    skipped_leaves: &[String],
    run_mode: CompileRunMode,
    expected_manifest_hash: &str,
) -> Result<CompileSummary, CompileError> {
    let tree = cfg.tree();
    recover_pending_if_needed(tree.path())?;

    // Load current manifest. Used to preserve branch `created_at` and carry
    // leaf records / tree metadata forward into the new manifest.
    let current = tree
        .manifest_or_empty_if_fresh()
        .map_err(|e| CompileError::Io(format!("failed to read manifest: {}", e)))?;

    let current_manifest_hash = pending::manifest_hash(tree.path())?;
    if current_manifest_hash != expected_manifest_hash {
        return Err(CompileError::Io(
            "manifest changed during compile planning; rerun `bo compile`".to_string(),
        ));
    }

    // Stale branches already repaired in pre-LLM pass; no deleted leaves remain.
    let delta = build_manifest_delta(&current, plan, run_mode, run_timestamp, &[], &[])?;

    let mut staged: Vec<StagedWrite> = Vec::new();
    for planned_write in &delta.branch_writes {
        let content = branch::format_content(
            planned_write.record.title.as_str(),
            &planned_write.body,
            &planned_write.file_leaves,
            &planned_write.record.created_at.to_rfc3339_millis(),
            &run_timestamp.to_rfc3339_millis(),
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
        tree.path(),
        OpKind::Compile { mode: compile_mode },
        writes.clone(),
        delta.branch_deletes.clone(),
    )?;
    let pending_path = pending::pending_path(tree.path());
    pending::write(&pending_path, &operation)?;
    for write in &staged {
        pending::write_staged(tree.path(), &write.pending, &write.bytes)?;
    }
    manifest::write(&tree.manifest_path(), &delta.new_manifest)
        .map_err(|e| CompileError::Io(format!("failed to write manifest: {}", e)))?;
    pending::apply_writes(tree.path(), &writes)?;
    pending::apply_deletes(tree.path(), &delta.branch_deletes)?;
    pending::clear(&pending_path)?;

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
