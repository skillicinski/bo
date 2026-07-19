// ── synthesis journal payloads ─────────────────────────────────────────────────

use std::time::Duration;

use serde::Serialize;

use super::repair;
use super::types::{BranchResult, SynthesisError, SynthesisMode, SynthesisStages};

#[derive(Serialize)]
pub(super) struct SynthesisJournalError {
    code: String,
    message: String,
}

#[derive(Serialize)]
pub(super) struct SynthesisJournalPayload<'a> {
    pub(super) mode: SynthesisMode,
    pub(super) new_leaf_slugs: &'a [String],
    pub(super) branches_created: &'a [BranchResult],
    pub(super) branches_updated: &'a [BranchResult],
    pub(super) branches_deleted: &'a [String],
    pub(super) validation_failures: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) error: Option<SynthesisJournalError>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) stages: Option<SynthesisStages>,
    pub(super) duration_ms: u128,
}

pub(super) fn synthesis_payload<'a>(
    summary: &'a super::types::SynthesisSummary,
    mode: SynthesisMode,
    new_leaf_slugs: &'a [String],
    duration: Duration,
    stages: Option<SynthesisStages>,
) -> SynthesisJournalPayload<'a> {
    SynthesisJournalPayload {
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

/// Build a synthesis journal event for a terminal write-path error, or `None`
/// when the error is not a synthesis outcome worth journaling (infrastructure
/// failures like Io/Busy, or the dry-run/agent paths which write zero bytes).
/// Validation keeps its own shape (`validation_failures`); LLM/provider
/// failures use `error: {code, message}` with empty deltas.
pub(super) fn error_payload<'a>(
    mode: SynthesisMode,
    new_leaf_slugs: &'a [String],
    error: &SynthesisError,
    duration: Duration,
) -> Option<SynthesisJournalPayload<'a>> {
    let (validation_failures, error_field) = match error {
        SynthesisError::Validation(msg) => (vec![msg.clone()], None),
        SynthesisError::Truncated
        | SynthesisError::ContentFilter
        | SynthesisError::Llm(_)
        | SynthesisError::ContextOverflow { .. } => {
            let json_error = error.json_error();
            (
                Vec::new(),
                Some(SynthesisJournalError {
                    code: json_error.code,
                    message: json_error.message,
                }),
            )
        }
        // Io/Busy/DryRunBlocked/AgentFailed: not synthesis verdicts.
        _ => return None,
    };
    Some(SynthesisJournalPayload {
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
#[path = "../../tests/cli_synthesize_journal_tests.rs"]
mod journal_tests;
