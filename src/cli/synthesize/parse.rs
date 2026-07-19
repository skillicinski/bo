// ── synthesis response parsing ─────────────────────────────────────────────────

use schemars::JsonSchema;
use serde::Deserialize;

use crate::engine::config::SeededConfig;

use super::plan;
use super::validation;
use super::SynthesisError;

// ── types ─────────────────────────────────────────────────────────────────────

/// Deserialized LLM response.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BranchSynthesisResponse {
    pub(super) branches: Vec<RawBranch>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct RawBranch {
    pub(super) title: String,
    pub(super) body: String,
    pub(super) leaves: Vec<String>,
}

/// Deserialized incremental LLM response.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct IncrementalSynthesisResponse {
    pub(super) updated_branches: Vec<RawUpdatedBranch>,
    pub(super) new_branches: Vec<RawBranch>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct RawUpdatedBranch {
    pub(super) slug: String,
    pub(super) title: String,
    pub(super) body: String,
    pub(super) leaves: Vec<String>,
}

// ── functions ─────────────────────────────────────────────────────────────────

pub(super) fn parse_and_validate_with_input_size(
    response: &str,
    loaded_leaves: &[plan::LoadedLeaf],
    input_body_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<validation::SynthesisPlan, SynthesisError> {
    let parsed: BranchSynthesisResponse = serde_json::from_str(response).map_err(|e| {
        SynthesisError::Validation(format!("invalid synthesis response shape: {}", e))
    })?;
    validation::validate_full(parsed, loaded_leaves, input_body_bytes, warnings)
}

pub(super) fn parse_incremental_response(
    response: &str,
) -> Result<IncrementalSynthesisResponse, SynthesisError> {
    serde_json::from_str(response).map_err(|e| {
        SynthesisError::Validation(format!(
            "invalid incremental synthesis response shape: {}",
            e
        ))
    })
}

pub(super) fn validate_incremental_response_with_input_size(
    parsed: IncrementalSynthesisResponse,
    cfg: &SeededConfig,
    loaded_leaves: &[plan::LoadedLeaf],
    input_body_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<validation::SynthesisPlan, SynthesisError> {
    let tree = cfg.tree();
    let state_path = crate::domain::tree::state_path(tree.path());
    let state = crate::engine::state::read(&state_path)
        .map_err(|e| SynthesisError::Io(format!("failed to read state: {}", e)))?;
    validation::validate_incremental(parsed, &state, loaded_leaves, input_body_bytes, warnings)
}

pub(super) fn parse_and_validate_incremental_with_input_size(
    response: &str,
    cfg: &SeededConfig,
    loaded_leaves: &[plan::LoadedLeaf],
    input_body_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<validation::SynthesisPlan, SynthesisError> {
    validate_incremental_response_with_input_size(
        parse_incremental_response(response)?,
        cfg,
        loaded_leaves,
        input_body_bytes,
        warnings,
    )
}

#[cfg(test)]
#[path = "../../tests/cli_synthesize_parse_tests.rs"]
mod parse_tests;
