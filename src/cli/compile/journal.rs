// ── compile journal payloads ─────────────────────────────────────────────────

use std::time::Duration;

use serde::Serialize;

use super::repair;
use super::types::{BranchResult, CompileError, CompileRunMode, CompileStages};

#[derive(Serialize)]
pub(super) struct CompileJournalError {
    code: String,
    message: String,
}

#[derive(Serialize)]
pub(super) struct CompileJournalPayload<'a> {
    pub(super) mode: CompileRunMode,
    pub(super) new_leaf_slugs: &'a [String],
    pub(super) branches_created: &'a [BranchResult],
    pub(super) branches_updated: &'a [BranchResult],
    pub(super) branches_deleted: &'a [String],
    pub(super) validation_failures: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<CompileJournalError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stages: Option<CompileStages>,
    pub(super) duration_ms: u128,
}

pub(super) fn compile_payload<'a>(
    summary: &'a super::types::CompileSummary,
    mode: CompileRunMode,
    new_leaf_slugs: &'a [String],
    duration: Duration,
    stages: Option<CompileStages>,
) -> CompileJournalPayload<'a> {
    CompileJournalPayload {
        mode,
        new_leaf_slugs,
        branches_created: &summary.branches_created,
        branches_updated: &summary.branches_updated,
        branches_deleted: &summary.branch_deletes,
        validation_failures: Vec::new(),
        error: None,
        stages,
        duration_ms: duration.as_millis(),
    }
}

/// Build a compile journal event for a terminal write-path error, or `None`
/// when the error is not a compile outcome worth journaling (infrastructure
/// failures like Io/Busy, or the dry-run/agent paths which write zero bytes).
/// Validation keeps its own shape (`validation_failures`); LLM/provider
/// failures use `error: {code, message}` with empty deltas.
pub(super) fn compile_error_payload<'a>(
    mode: CompileRunMode,
    new_leaf_slugs: &'a [String],
    error: &CompileError,
    duration: Duration,
) -> Option<CompileJournalPayload<'a>> {
    let (validation_failures, error_field) = match error {
        CompileError::Validation(msg) => (vec![msg.clone()], None),
        CompileError::Truncated
        | CompileError::ContentFilter
        | CompileError::Llm(_)
        | CompileError::ContextOverflow { .. } => {
            let json_error = error.json_error();
            (
                Vec::new(),
                Some(CompileJournalError {
                    code: json_error.code,
                    message: json_error.message,
                }),
            )
        }
        // Io/Busy/DryRunBlocked/AgentFailed: not compile verdicts.
        _ => return None,
    };
    Some(CompileJournalPayload {
        mode,
        new_leaf_slugs,
        branches_created: &[],
        branches_updated: &[],
        branches_deleted: &[],
        validation_failures,
        error: error_field,
        stages: None,
        duration_ms: duration.as_millis(),
    })
}

#[derive(Serialize)]
pub(super) struct RepairJournalPayload<'a> {
    pub(super) orphan_leaf_slugs: &'a [String],
    pub(super) repaired_branch_slugs: &'a [String],
    pub(super) removed_branches: &'a [repair::RemovedBranchResult],
}

pub(super) fn repair_journal_payload(report: &repair::RepairReport) -> RepairJournalPayload<'_> {
    RepairJournalPayload {
        orphan_leaf_slugs: &report.orphan_leaf_slugs,
        repaired_branch_slugs: &report.repaired_branch_slugs,
        removed_branches: &report.removed_branches,
    }
}

#[cfg(test)]
#[path = "../../tests/cli_compile_journal_tests.rs"]
mod journal_tests;
