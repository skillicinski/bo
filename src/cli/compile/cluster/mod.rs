// ── two-stage compile: cluster stage modules ─────────────────────────────────
//
// Stage 1 (cluster): deterministic pre-pass over leaf bodies to produce
// candidate groupings, then one LLM call over titles+summaries that adjudicates
// and names clusters. Stage 2 (synthesize) reuses the existing per-branch
// parse/validate/execute path.

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

#[cfg(test)]
mod tests {
    #[test]
    fn two_stage_threshold_values() {
        assert_eq!(crate::cli::compile::TWO_STAGE_FULL_THRESHOLD, 40);
        assert_eq!(crate::cli::compile::TWO_STAGE_INCREMENTAL_THRESHOLD, 15);
    }
}
