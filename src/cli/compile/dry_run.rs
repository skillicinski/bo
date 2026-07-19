// ── compile dry-run: read-only preflight and preview ───────────────────────

use crate::domain::state;
use crate::domain::tree::TreeLoadState;
use crate::engine::auth;
use crate::engine::config::SeededConfig;
use crate::engine::llm::{LlmProvider, Model};
use crate::engine::transaction;

use super::agent;
use super::plan;
use super::repair;
use super::types::{
    CompileDryRunOutcome, CompileError, CompileOptions, CompilePreview, CompileRunMode,
    PreviewBranch, NO_NEW_LEAVES_REASON,
};

struct DryRunRequest {
    state: state::TreeState,
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
    let preflight = dry_run_preflight(cfg, options);
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
        let provider = crate::engine::llm::create_provider(
            cfg.config.provider,
            &api_key,
            cfg.config.base_url.as_deref(),
        )
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

/// Testable dry-run seam with an injected provider and model.
pub fn run_compile_dry_run_with_provider(
    cfg: &SeededConfig,
    options: CompileOptions,
    provider: &dyn LlmProvider,
    model: &Model,
) -> CompileDryRunOutcome {
    let mut warnings = Vec::new();
    let preflight = dry_run_preflight(cfg, options);
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
) -> Result<DryRunPreflight, CompileError> {
    let tree = cfg.tree();
    let tree_dir = tree.path();

    // ZERO writes: read-only pending check. Do not recover.
    if transaction::read(&transaction::pending_path(tree_dir))?.is_some() {
        return Err(CompileError::DryRunBlocked(
            "an unfinished transaction exists; run `bo compile` (without --dry-run) to recover it before previewing".to_string(),
        ));
    }

    let state = match crate::engine::state::load_state(tree_dir) {
        Ok(TreeLoadState::Loaded(state)) => state,
        Ok(TreeLoadState::FreshSeeded) => {
            return Ok(DryRunPreflight::Noop(noop_preview(
                "empty_tree",
                cfg,
                "<missing>",
                options.agent,
            )));
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

    // ZERO writes: read-only stale-repair check. Do not repair.
    if repair::requires_repair(cfg, &state)? {
        return Err(CompileError::DryRunBlocked(
            "stale branches require repair; run `bo compile` (without --dry-run) to repair before previewing".to_string(),
        ));
    }

    // Capture the state hash at start; recheck before accepting the preview.
    let starting_hash = transaction::state_hash(tree_dir)?;

    let run_mode = plan::select_run_mode(options, &state);
    if state.leaves.is_empty() {
        return Ok(DryRunPreflight::Noop(noop_preview(
            "empty_tree",
            cfg,
            &starting_hash,
            options.agent,
        )));
    }
    let new_leaf_slugs = plan::select_new_leaf_slugs(&state)?;
    if !options.all && new_leaf_slugs.is_empty() {
        return Ok(DryRunPreflight::Noop(noop_preview(
            NO_NEW_LEAVES_REASON,
            cfg,
            &starting_hash,
            options.agent,
        )));
    }
    let (loaded_leaves, skipped_leaves) = plan::read_valid_leaves(cfg, &state.leaves);
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
        state,
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
        state,
        loaded_leaves,
        skipped_leaves,
        new_leaf_slugs,
        run_mode,
        starting_hash,
    } = req;

    let (plan, stats, validation_warnings) = if options.agent {
        let (plan, stats, vw) =
            agent::run_agent_dry_run(cfg, provider, model, &state, &loaded_leaves, run_mode)?;
        (plan, stats, vw)
    } else {
        let (plan, compile_stages) = plan::build_compile_plan(
            cfg,
            provider,
            model,
            &state,
            &loaded_leaves,
            &new_leaf_slugs,
            run_mode,
            warnings,
        )?;
        let stats = agent::AgentRunStats {
            turns: 1 + compile_stages.as_ref().map_or(0, |s| s.stage2_calls),
            tool_calls: 0,
            usage: None,
        };
        (plan, stats, Vec::new())
    };
    warnings.extend(validation_warnings);

    // Recheck the state hash; abort if the tree changed mid-run.
    let current_hash = transaction::state_hash(cfg.tree().path())?;
    let state_unchanged = current_hash == starting_hash;
    if !state_unchanged {
        return Err(CompileError::DryRunBlocked(
            "state changed during dry-run; rerun `bo compile --dry-run`".to_string(),
        ));
    }
    Ok(CompilePreview {
        status: "preview".to_string(),
        reason: None,
        mode: Some(run_mode),
        provider: cfg.config.provider.to_string(),
        model: model.to_string(),
        starting_state_hash: starting_hash,
        state_unchanged,
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
        starting_state_hash: starting_hash.to_string(),
        state_unchanged: true,
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
