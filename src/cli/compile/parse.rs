// ── compile response parsing ─────────────────────────────────────────────────

use schemars::JsonSchema;
use serde::Deserialize;

use crate::engine::config::SeededConfig;

use super::plan;
use super::validation;
use super::CompileError;

// ── types ─────────────────────────────────────────────────────────────────────

/// Deserialized LLM response.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct CompileResponse {
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
pub(super) struct IncrementalCompileResponse {
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
) -> Result<validation::CompilePlan, CompileError> {
    let parsed: CompileResponse = serde_json::from_str(response)
        .map_err(|e| CompileError::Validation(format!("invalid compile response shape: {}", e)))?;
    validation::validate_full(parsed, loaded_leaves, input_body_bytes, warnings)
}

pub(super) fn parse_incremental_response(
    response: &str,
) -> Result<IncrementalCompileResponse, CompileError> {
    serde_json::from_str(response).map_err(|e| {
        CompileError::Validation(format!("invalid incremental compile response shape: {}", e))
    })
}

pub(super) fn validate_incremental_response_with_input_size(
    parsed: IncrementalCompileResponse,
    cfg: &SeededConfig,
    loaded_leaves: &[plan::LoadedLeaf],
    input_body_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<validation::CompilePlan, CompileError> {
    let tree = cfg.tree();
    let manifest_path = crate::domain::tree::manifest_path(tree.path());
    let manifest = crate::engine::manifest::read(&manifest_path)
        .map_err(|e| CompileError::Io(format!("failed to read manifest: {}", e)))?;
    validation::validate_incremental(parsed, &manifest, loaded_leaves, input_body_bytes, warnings)
}

pub(super) fn parse_and_validate_incremental_with_input_size(
    response: &str,
    cfg: &SeededConfig,
    loaded_leaves: &[plan::LoadedLeaf],
    input_body_bytes: usize,
    warnings: &mut Vec<String>,
) -> Result<validation::CompilePlan, CompileError> {
    validate_incremental_response_with_input_size(
        parse_incremental_response(response)?,
        cfg,
        loaded_leaves,
        input_body_bytes,
        warnings,
    )
}

#[cfg(test)]
#[path = "../../tests/cli_compile_parse_tests.rs"]
mod parse_tests;
