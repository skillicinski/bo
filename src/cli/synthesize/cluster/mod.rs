// ── two-stage synthesis: cluster stage modules ───────────────────────────────
//
// Stage 1 (cluster): deterministic pre-pass over leaf bodies to produce
// candidate groupings, then one LLM call over titles+summaries that adjudicates
// and names clusters. Stage 2 (synthesize) reuses the existing per-branch
// parse/validate/execute path.

use crate::domain::state;
use crate::engine::config::SeededConfig;
use crate::engine::llm::{LlmProvider, Model};

use super::execute;
use super::plan::LoadedLeaf;
use super::types::{SynthesisError, SynthesisStages};
use super::validation::SynthesisPlan;

mod discovery;
mod prompt;
mod synthesize;
mod validation;

#[cfg(test)]
#[path = "validation_tests.rs"]
mod validation_tests;

pub(super) use prompt::{
    build_cluster_user_message, build_incremental_cluster_user_message, CLUSTER_SYSTEM_PROMPT,
    INCREMENTAL_CLUSTER_SYSTEM_PROMPT,
};
pub(super) use synthesize::{
    plan_from_stage2, run_stage2_synthesize, run_stage2_synthesize_incremental,
};
pub(super) use validation::{
    validate_clusters, validate_incremental_clusters, ClusterResponse, IncrementalClusterResponse,
};

// ── two-stage drivers ────────────────────────────────────────────────────────

/// Run two-stage synthesis for Full mode: cluster → per-cluster synthesis → plan.
pub(in crate::cli::synthesize) fn run_two_stage_full(
    _cfg: &SeededConfig,
    provider: &dyn LlmProvider,
    model: &Model,
    loaded_leaves: &[LoadedLeaf],
    warnings: &mut Vec<String>,
) -> Result<(SynthesisPlan, SynthesisStages), SynthesisError> {
    // Stage 1: Cluster pass (titles + summaries, one LLM call).
    let cluster_user_message = build_cluster_user_message(loaded_leaves);
    let cluster_tokens = execute::estimate_synthesis_prompt_tokens(
        CLUSTER_SYSTEM_PROMPT
            .len()
            .saturating_add(cluster_user_message.len()),
    );
    execute::ensure_synthesis_context_fits(model, cluster_tokens)?;
    let cluster_schema =
        serde_json::to_value(crate::engine::schema::inline_schema_for::<ClusterResponse>())
            .unwrap();
    let cluster_response = execute::call_llm_blocking(
        provider,
        model,
        &cluster_user_message,
        &cluster_schema,
        CLUSTER_SYSTEM_PROMPT,
    )?;
    let parsed: ClusterResponse = serde_json::from_str(&cluster_response).map_err(|e| {
        SynthesisError::Validation(format!("invalid cluster response shape: {}", e))
    })?;
    let stage1 = validate_clusters(&parsed, loaded_leaves, warnings)?;
    let cluster_count = stage1.clusters.len();

    // Stage 2: Per-cluster synthesize (one LLM call each).
    let branches = run_stage2_synthesize(provider, model, &stage1, loaded_leaves, warnings)?;

    let plan = SynthesisPlan { branches };
    let stages = SynthesisStages {
        stage1_clusters: cluster_count,
        stage2_calls: cluster_count, // ponytail: one call per cluster, always matches cluster_count
    };
    Ok((plan, stages))
}

/// Run two-stage synthesis for Incremental mode: cluster new leaves against
/// existing branches → per-cluster synthesize (updates + new) → plan.
pub(in crate::cli::synthesize) fn run_two_stage_incremental(
    cfg: &SeededConfig,
    provider: &dyn LlmProvider,
    model: &Model,
    state: &state::TreeState,
    loaded_leaves: &[LoadedLeaf],
    new_leaf_slugs: &[String],
    warnings: &mut Vec<String>,
) -> Result<(SynthesisPlan, SynthesisStages), SynthesisError> {
    // Stage 1: Cluster new leaves against existing branch titles + summaries.
    let cluster_user_message =
        build_incremental_cluster_user_message(cfg, state, loaded_leaves, new_leaf_slugs);
    let cluster_tokens = execute::estimate_synthesis_prompt_tokens(
        INCREMENTAL_CLUSTER_SYSTEM_PROMPT
            .len()
            .saturating_add(cluster_user_message.len()),
    );
    execute::ensure_synthesis_context_fits(model, cluster_tokens)?;
    let cluster_schema = serde_json::to_value(crate::engine::schema::inline_schema_for::<
        IncrementalClusterResponse,
    >())
    .unwrap();
    let cluster_response = execute::call_llm_blocking(
        provider,
        model,
        &cluster_user_message,
        &cluster_schema,
        INCREMENTAL_CLUSTER_SYSTEM_PROMPT,
    )?;
    let parsed: IncrementalClusterResponse =
        serde_json::from_str(&cluster_response).map_err(|e| {
            SynthesisError::Validation(format!("invalid incremental cluster response shape: {}", e))
        })?;
    let stage1 = validate_incremental_clusters(&parsed, state, loaded_leaves, warnings)?;
    let cluster_count = stage1.clusters.len();

    // Stage 2: Per-cluster synthesize (updates for existing branches, fresh for new).
    let (updated, new) = run_stage2_synthesize_incremental(
        cfg,
        provider,
        model,
        &stage1,
        state,
        loaded_leaves,
        warnings,
    )?;

    let plan = plan_from_stage2(updated, new);
    let stages = SynthesisStages {
        stage1_clusters: cluster_count,
        stage2_calls: cluster_count, // ponytail: one call per cluster, always matches cluster_count
    };
    Ok((plan, stages))
}

#[cfg(test)]
mod tests {
    #[test]
    fn two_stage_threshold_values() {
        assert_eq!(crate::cli::synthesize::types::TWO_STAGE_FULL_THRESHOLD, 40);
        assert_eq!(
            crate::cli::synthesize::types::TWO_STAGE_INCREMENTAL_THRESHOLD,
            15
        );
    }
}
