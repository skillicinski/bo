// ── compile orchestration: live preflight, write-path coordination ──────────

use std::collections::HashSet;

use crate::domain::state;
use crate::domain::tree::{self, TreeLoadState};
use crate::domain::Timestamp;
use crate::engine::auth;
use crate::engine::config::SeededConfig;
use crate::engine::journal;
use crate::engine::llm::{LlmProvider, Model};
use crate::engine::transaction;

use super::dry_run::run_compile_dry_run;
use super::execute;
use super::journal as journal_mod;
use super::plan;
use super::repair;
use super::types::{
    BranchResult, CompileDryRunOutcome, CompileError, CompileOptions, CompileOutcome,
    CompileResult, CompileRunMode, CompileSummary, NO_NEW_LEAVES_REASON,
};

pub(super) fn preflight_noop(
    state: &state::TreeState,
    _options: CompileOptions,
    notifications: &[String],
) -> Option<CompileResult> {
    match state.leaves.len() {
        0 => return Some(CompileResult::noop("empty_tree", notifications.to_vec())),
        1 => return Some(CompileResult::noop("single_leaf", notifications.to_vec())),
        _ => {}
    }
    None
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

fn run_compile(
    cfg: &SeededConfig,
    options: CompileOptions,
    warnings: &mut Vec<String>,
) -> Result<CompileResult, CompileError> {
    let compile_started_at = Timestamp::now();

    // Stale repair runs before preflight so preflight sees repaired state.
    let tree = cfg.tree();
    execute::recover_transaction_if_needed(tree.path(), warnings)?;
    let state = match crate::engine::state::load_state(tree.path()) {
        Ok(TreeLoadState::Loaded(state)) => state,
        Ok(TreeLoadState::FreshSeeded) => {
            return Ok(CompileResult::noop("empty_tree", Vec::new()));
        }
        Ok(TreeLoadState::MissingState) => {
            return Err(CompileError::Io(format!(
                "failed to read state: {}",
                state::TreeStateError::TreeNotInitialized
            )));
        }
        Err(error) => {
            return Err(CompileError::Io(format!("failed to read state: {}", error)));
        }
    };
    let repair_report = repair::repair_stale_branches(cfg, &state)?;
    // Repair notices are destructive-action reporting (e.g. "removed N stale
    // branches"); mirror them onto the stderr channel so they reach consumers
    // in both human and --json mode. The human-mode double-emission (stderr
    // line + stdout `→` note) matches the prior behavior byte-for-byte.
    warnings.extend(repair_report.notifications.iter().cloned());
    if !repair_report.is_empty() {
        journal::append_payload(
            tree.path(),
            journal::Op::Repair,
            None,
            &journal_mod::repair_journal_payload(&repair_report),
        );
    }
    let notifications = repair_report.notifications;
    let state = crate::engine::state::read(&tree::state_path(tree.path()))
        .map_err(|e| CompileError::Io(format!("failed to read state: {}", e)))?;

    if let Some(noop) = preflight_noop(&state, options, &notifications) {
        return Ok(noop);
    }
    let new_leaf_slugs = plan::select_new_leaf_slugs(&state)?;
    if !options.all && new_leaf_slugs.is_empty() {
        return Ok(CompileResult::noop(NO_NEW_LEAVES_REASON, notifications));
    }

    let expected_state_hash = transaction::state_hash(tree.path())?;

    let api_key =
        auth::resolve_api_key(cfg.config.provider).map_err(|e| CompileError::Llm(e.to_string()))?;
    let provider = crate::engine::llm::create_provider(
        cfg.config.provider,
        &api_key,
        cfg.config.base_url.as_deref(),
    )
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
        &state,
        &new_leaf_slugs,
        &expected_state_hash,
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
    compile_started_at: &Timestamp,
    mut notifications: Vec<String>,
    state: &state::TreeState,
    new_leaf_slugs: &[String],
    expected_state_hash: &str,
    warnings: &mut Vec<String>,
) -> Result<CompileResult, CompileError> {
    let (loaded_leaves, skipped_leaves) = plan::read_valid_leaves(cfg, &state.leaves);

    if loaded_leaves.len() < 2 {
        if loaded_leaves.is_empty() {
            return Ok(CompileResult::noop("empty_tree", notifications));
        }
        return Ok(CompileResult::noop("single_leaf", notifications));
    }

    let tree = cfg.tree();
    let started = std::time::Instant::now();

    let run_mode = plan::select_run_mode(options, state);
    let valid_filenames: HashSet<String> =
        loaded_leaves.iter().map(|l| l.filename.clone()).collect();
    let run_timestamp = compile_started_at;

    let mut compile_stages: Option<super::types::CompileStages> = None;

    let outcome = (|| -> Result<CompileSummary, CompileError> {
        let (compiled_plan, stages) = plan::build_compile_plan(
            cfg,
            provider,
            model,
            state,
            &loaded_leaves,
            new_leaf_slugs,
            run_mode,
            warnings,
        )?;
        compile_stages = stages;
        execute::execute_plan_with_mode_and_expected_hash(
            &compiled_plan,
            cfg,
            &valid_filenames,
            run_timestamp,
            &skipped_leaves,
            run_mode,
            expected_state_hash,
            warnings,
        )
    })();

    match outcome {
        Ok(summary) => {
            if let Some(warning) = degenerate_result_warning(
                Some(run_mode),
                &summary.branches,
                summary.leaves_processed,
            ) {
                notifications.push(warning);
            }
            journal::append_payload(
                tree.path(),
                journal::Op::Compile,
                Some(model.to_string()),
                &journal_mod::compile_payload(
                    &summary,
                    run_mode,
                    new_leaf_slugs,
                    started.elapsed(),
                    compile_stages,
                ),
            );
            Ok(CompileResult::compiled(
                summary,
                run_mode,
                model,
                notifications,
            ))
        }
        Err(error) => {
            if let Some(payload) = journal_mod::compile_error_payload(
                run_mode,
                new_leaf_slugs,
                &error,
                started.elapsed(),
            ) {
                journal::append_payload(
                    tree.path(),
                    journal::Op::Compile,
                    Some(model.to_string()),
                    &payload,
                );
            }
            Err(error)
        }
    }
}

// ── entry point: dry-run / live dispatch ────────────────────────────────────

pub enum Dispatch {
    DryRun(CompileDryRunOutcome),
    Live(CompileOutcome),
}

pub fn run(cfg: &SeededConfig, options: CompileOptions) -> Dispatch {
    if options.dry_run {
        Dispatch::DryRun(run_compile_dry_run(cfg, options))
    } else {
        Dispatch::Live(run_compile_with_options(cfg, options))
    }
}
