// ── compile execution: LLM calls, staging, disk writes ────────────────────────

use std::collections::HashSet;

use serde_json::Value;

use crate::domain::{branch, Timestamp};

use crate::engine::config::SeededConfig;
use crate::engine::llm::{
    complete_with_policy, FinishReason, LlmCallPolicy, LlmError, LlmProvider, Message, Model,
};
use crate::engine::pending::{self, CompileMode, OpKind, PendingWrite};

use super::plan::build_state_delta;
use super::types::{
    COMPILE_LLM_POLICY, COMPILE_PROMPT_OVERHEAD_TOKENS, MAX_COMPLETION_TOKENS,
    TOKEN_ESTIMATE_BYTES_PER_TOKEN,
};
use super::validation::CompilePlan;
use super::{BranchResult, CompileError, CompileRunMode, CompileSummary};

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

// ── LLM call ──────────────────────────────────────────────────────────────────

pub(super) fn call_llm_blocking(
    provider: &dyn LlmProvider,
    model: &Model,
    user_message: &str,
    schema: &Value,
    system_prompt: &str,
) -> Result<String, CompileError> {
    crate::engine::llm::blocking_runtime().block_on(call_llm_with_provider(
        provider,
        model.as_str(),
        user_message,
        schema,
        COMPILE_LLM_POLICY,
        system_prompt,
    ))
}

async fn call_llm_with_provider(
    provider: &dyn LlmProvider,
    model: &str,
    user_message: &str,
    schema: &Value,
    policy: LlmCallPolicy,
    system_prompt: &str,
) -> Result<String, CompileError> {
    let messages = vec![Message::system(system_prompt), Message::user(user_message)];

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
        _ if message.contains("maximum context length") => CompileError::ContextOverflow {
            model: "unknown".to_string(),
            estimated_tokens: None,
            context_tokens: None,
        },
        other => CompileError::Llm(other.to_string()),
    }
}

// ── pending/recovery ──────────────────────────────────────────────────────────

pub(super) fn recover_pending_if_needed(
    path: &std::path::Path,
    warnings: &mut Vec<String>,
) -> Result<(), CompileError> {
    if let Some(report) = pending::recover_or_refuse(path)? {
        warnings.push(format!(
            "recovered {} changes from interrupted {}",
            report.changes, report.op
        ));
    }
    Ok(())
}

// ── execute plan ──────────────────────────────────────────────────────────────

// ponytail: 8 args; collapse into an execution-context struct if it grows.
#[allow(clippy::too_many_arguments)]
pub(super) fn execute_plan_with_mode_and_expected_hash(
    plan: &CompilePlan,
    cfg: &SeededConfig,
    valid_filenames: &HashSet<String>,
    run_timestamp: &Timestamp,
    skipped_leaves: &[String],
    run_mode: CompileRunMode,
    expected_state_hash: &str,
    warnings: &mut Vec<String>,
) -> Result<CompileSummary, CompileError> {
    let tree = cfg.tree();
    let tree_dir = tree.path();
    recover_pending_if_needed(tree_dir, warnings)?;

    // Load current state. Used to preserve branch `created_at` and carry
    // leaf records / tree metadata forward into the new state.
    let current = crate::engine::state::state_or_empty_if_fresh(&tree)
        .map_err(|e| CompileError::Io(format!("failed to read state: {}", e)))?;

    let current_state_hash = pending::state_hash(tree_dir)?;
    if current_state_hash != expected_state_hash {
        return Err(CompileError::Io(
            "state changed during compile planning; rerun `bo compile`".to_string(),
        ));
    }

    // Stale branches already repaired in pre-LLM pass; no deleted leaves remain.
    let delta = build_state_delta(&current, plan, run_mode, run_timestamp)?;

    let mut staged: Vec<(PendingWrite, Vec<u8>)> = Vec::new();
    for planned_write in &delta.branch_writes {
        let content = branch::format_content(
            &planned_write.record.title,
            &planned_write.body,
            &planned_write.file_leaves,
            &planned_write.record.created_at,
            run_timestamp,
        );
        let bytes = content.into_bytes();
        staged.push((
            PendingWrite {
                path: planned_write.record.file.clone(),
                content_hash: pending::content_hash(&bytes),
            },
            bytes,
        ));
    }

    let leaves_processed = valid_filenames.len();

    let compile_mode = match run_mode {
        CompileRunMode::Incremental => CompileMode::Incremental,
        CompileRunMode::Full => CompileMode::Full,
    };
    let staged_refs: Vec<(&PendingWrite, &[u8])> =
        staged.iter().map(|(pw, b)| (pw, b.as_slice())).collect();
    pending::commit_with_state(
        tree_dir,
        OpKind::Compile { mode: compile_mode },
        &delta.new_state,
        &staged_refs,
        &delta.branch_deletes,
    )
    .map_err(|e| CompileError::Io(format!("failed to commit compile: {}", e)))?;

    let branch_results: Vec<BranchResult> = delta
        .branches_created
        .iter()
        .chain(delta.branches_updated.iter())
        .cloned()
        .collect();

    for branch in &branch_results {
        warnings.push(format!("writing branch: {}", branch.slug));
    }

    Ok(CompileSummary {
        branches: branch_results,
        branches_created: delta.branches_created,
        branches_updated: delta.branches_updated,
        branch_deletes: delta.branch_deletes,
        leaves_processed,
        leaves_skipped: skipped_leaves.to_vec(),
    })
}

#[cfg(test)]
#[path = "../../tests/cli_compile_execute_tests.rs"]
mod execute_tests;
