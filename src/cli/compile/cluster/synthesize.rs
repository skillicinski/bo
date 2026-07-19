// ── two-stage compile: stage 2 per-cluster synthesize ────────────────────────

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::Deserialize;

use crate::cli::compile::execute;
use crate::cli::compile::plan::LoadedLeaf;
use crate::cli::compile::prompt::COMPILE_SYSTEM_PROMPT;
use crate::cli::compile::validation::{CompilePlan, ValidatedBranch};
use crate::cli::compile::CompileError;
use crate::domain::state::TreeState;
use crate::engine::config::SeededConfig;
use crate::engine::schema::inline_schema_for;

use super::prompt::{build_synthesize_update_user_message, build_synthesize_user_message};
use super::validation::ValidatedClusters;

// ── stage 2 synthesize schema (body-only) ───────────────────────────────────

/// Stage-2 LLM response: title + body only. Membership is constructed from the
/// validated cluster, not from the model output. This eliminates the
/// "branch references unknown leaf" failure class entirely.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct Stage2Response {
    title: String,
    body: String,
}

// ── stage 2: per-cluster synthesize ──────────────────────────────────────────

/// Run stage 2 for a set of validated clusters in Full mode.
/// Each cluster gets one LLM call (body-only schema). Membership is taken
/// from the validated cluster, not the model output.
///
/// Retry: one retry on transport/shape failure per cluster; on second failure,
/// drop cluster with warning. Hard error only when ALL clusters fail.
///
/// ponytail: sequential calls for v1; parallelize with tokio::spawn if
/// per-cluster latency becomes the bottleneck.
pub(in crate::cli::compile) fn run_stage2_synthesize(
    provider: &dyn crate::engine::llm::LlmProvider,
    model: &crate::engine::llm::Model,
    clusters: &ValidatedClusters,
    all_leaves: &[LoadedLeaf],
    warnings: &mut Vec<String>,
) -> Result<Vec<ValidatedBranch>, CompileError> {
    let schema = serde_json::to_value(inline_schema_for::<Stage2Response>()).unwrap();
    let mut all_branches: Vec<ValidatedBranch> = Vec::new();
    let mut seen_slugs: HashSet<String> = HashSet::new();
    let mut any_success = false;

    for cluster in &clusters.clusters {
        let cluster_leaves: Vec<&LoadedLeaf> = cluster
            .leaf_files
            .iter()
            .filter_map(|f| all_leaves.iter().find(|l| l.filename == *f))
            .collect();

        let user_message = build_synthesize_user_message(&cluster.title, &cluster_leaves);
        let prompt_tokens = execute::estimate_compile_prompt_tokens(
            COMPILE_SYSTEM_PROMPT
                .len()
                .saturating_add(user_message.len()),
        );
        execute::ensure_compile_context_fits(model, prompt_tokens)?;

        match synthesize_one_cluster(
            provider,
            model,
            &schema,
            &user_message,
            &cluster.title,
            &cluster.leaf_files,
            warnings,
        ) {
            Ok(branch) => {
                any_success = true;
                let slug = crate::domain::Slug::generate(&branch.title, "").to_string();
                if seen_slugs.insert(slug.clone()) {
                    all_branches.push(branch);
                } else {
                    warnings.push(format!(
                        "warning: cluster '{}' produced duplicate slug '{}' — skipping",
                        cluster.title, slug
                    ));
                }
            }
            Err(_) => {
                // Failure already warned inside synthesize_one_cluster.
            }
        }
    }

    if !any_success {
        return Err(CompileError::Validation(
            "stage-2 synthesize: all clusters failed — no branches produced".to_string(),
        ));
    }

    Ok(all_branches)
}

/// Run stage 2 for incremental mode: existing-branch assignments get update
/// calls; new clusters get fresh synthesize calls.
///
/// Retry: same per-cluster retry semantics as Full mode.
///
/// Returns (updated_branches, new_branches).
pub(in crate::cli::compile) fn run_stage2_synthesize_incremental(
    cfg: &SeededConfig,
    provider: &dyn crate::engine::llm::LlmProvider,
    model: &crate::engine::llm::Model,
    clusters: &ValidatedClusters,
    state: &TreeState,
    all_leaves: &[LoadedLeaf],
    warnings: &mut Vec<String>,
) -> Result<(Vec<ValidatedBranch>, Vec<ValidatedBranch>), CompileError> {
    let schema = serde_json::to_value(inline_schema_for::<Stage2Response>()).unwrap();
    let tree = cfg.tree();
    let mut updated_branches: Vec<ValidatedBranch> = Vec::new();
    let mut new_branches: Vec<ValidatedBranch> = Vec::new();
    let mut seen_slugs: HashSet<String> = HashSet::new();
    let mut any_success = false;

    for cluster in &clusters.clusters {
        let cluster_leaves: Vec<&LoadedLeaf> = cluster
            .leaf_files
            .iter()
            .filter_map(|f| all_leaves.iter().find(|l| l.filename == *f))
            .collect();

        if cluster.is_existing_branch() {
            // Read existing branch body for the update prompt.
            let existing = match state.branch_by_slug_str(&cluster.existing_branch_slug) {
                Some(b) => b,
                None => {
                    warnings.push(format!(
                        "warning: existing branch '{}' not found — skipping update",
                        cluster.existing_branch_slug
                    ));
                    continue;
                }
            };

            let branch_path = tree.join(&existing.file);
            let existing_body = std::fs::read_to_string(&branch_path)
                .ok()
                .and_then(|content| {
                    crate::domain::frontmatter::parse(&content)
                        .ok()
                        .map(|(_, body)| body)
                })
                .unwrap_or_default();

            let user_message = build_synthesize_update_user_message(
                existing.title.as_str(),
                &existing_body,
                &cluster_leaves,
            );
            let prompt_tokens = execute::estimate_compile_prompt_tokens(
                COMPILE_SYSTEM_PROMPT
                    .len()
                    .saturating_add(user_message.len()),
            );
            execute::ensure_compile_context_fits(model, prompt_tokens)?;

            // Merge cluster leaves with existing branch leaves.
            let mut all_leaf_files: Vec<String> = cluster.leaf_files.clone();
            for leaf_slug in &existing.leaves {
                if let Some(leaf) = state.leaves.iter().find(|l| l.slug == *leaf_slug) {
                    let f = leaf.file.clone();
                    if !all_leaf_files.contains(&f) {
                        all_leaf_files.push(f);
                    }
                }
            }

            match synthesize_one_cluster(
                provider,
                model,
                &schema,
                &user_message,
                &format!("update for '{}'", existing.slug),
                &all_leaf_files,
                warnings,
            ) {
                Ok(mut branch) => {
                    any_success = true;
                    // Override slug/title with existing branch values.
                    branch.slug = existing.slug.as_str().to_string();
                    branch.title = existing.title.as_str().to_string();
                    if seen_slugs.insert(branch.slug.clone()) {
                        updated_branches.push(branch);
                    }
                }
                Err(_) => {
                    warnings.push(format!(
                        "warning: update for branch '{}' failed — keeping existing body",
                        existing.slug
                    ));
                }
            }
        } else {
            // New cluster — same as Full mode.
            let user_message = build_synthesize_user_message(&cluster.title, &cluster_leaves);
            let prompt_tokens = execute::estimate_compile_prompt_tokens(
                COMPILE_SYSTEM_PROMPT
                    .len()
                    .saturating_add(user_message.len()),
            );
            execute::ensure_compile_context_fits(model, prompt_tokens)?;

            match synthesize_one_cluster(
                provider,
                model,
                &schema,
                &user_message,
                &cluster.title,
                &cluster.leaf_files,
                warnings,
            ) {
                Ok(branch) => {
                    any_success = true;
                    let slug = crate::domain::Slug::generate(&branch.title, "").to_string();
                    if seen_slugs.insert(slug.clone()) {
                        new_branches.push(branch);
                    } else {
                        warnings.push(format!(
                            "warning: cluster '{}' produced duplicate slug '{}' — skipping",
                            cluster.title, slug
                        ));
                    }
                }
                Err(_) => {
                    // Failure already warned inside synthesize_one_cluster.
                }
            }
        }
    }

    if !any_success {
        return Err(CompileError::Validation(
            "stage-2 synthesize: all clusters failed — no branches produced".to_string(),
        ));
    }

    Ok((updated_branches, new_branches))
}

/// Call the LLM for one cluster, retrying once on transport/shape failure.
/// Returns the validated branch on success.
///
/// On persistent failure: warns and returns Err. The caller decides whether
/// to abort (all clusters failed) or continue (partial success).
fn synthesize_one_cluster(
    provider: &dyn crate::engine::llm::LlmProvider,
    model: &crate::engine::llm::Model,
    schema: &serde_json::Value,
    user_message: &str,
    cluster_label: &str,
    leaf_files: &[String],
    warnings: &mut Vec<String>,
) -> Result<ValidatedBranch, CompileError> {
    // Attempt 1.
    let response =
        execute::call_llm_blocking(provider, model, user_message, schema, COMPILE_SYSTEM_PROMPT);
    match parse_stage2_response(&response, cluster_label, leaf_files) {
        Ok(branch) => Ok(branch),
        Err(_first_error) => {
            // Attempt 2 (retry once).
            let retry_response = execute::call_llm_blocking(
                provider,
                model,
                user_message,
                schema,
                COMPILE_SYSTEM_PROMPT,
            );
            match parse_stage2_response(&retry_response, cluster_label, leaf_files) {
                Ok(branch) => {
                    warnings.push(format!(
                        "warning: cluster '{}' succeeded on retry",
                        cluster_label
                    ));
                    Ok(branch)
                }
                Err(second_error) => {
                    warnings.push(format!(
                        "warning: cluster '{}' failed after retry — dropping (error: {})",
                        cluster_label, second_error
                    ));
                    Err(second_error)
                }
            }
        }
    }
}

/// Parse a stage-2 LLM response (title+body only) and construct a
/// ValidatedBranch with membership taken from the cluster's leaf_files.
pub(super) fn parse_stage2_response(
    response: &Result<String, CompileError>,
    cluster_label: &str,
    leaf_files: &[String],
) -> Result<ValidatedBranch, CompileError> {
    let response = match response {
        Ok(r) => r,
        Err(e) => {
            return Err(CompileError::Validation(format!(
                "stage-2 LLM call failed for '{}': {}",
                cluster_label, e
            )));
        }
    };

    let parsed: Stage2Response = serde_json::from_str(response).map_err(|e| {
        CompileError::Validation(format!(
            "invalid stage-2 response for '{}': {}",
            cluster_label, e
        ))
    })?;

    let title = parsed.title.trim().to_string();
    if title.is_empty() {
        return Err(CompileError::Validation(format!(
            "invalid stage-2 response for '{}': empty title",
            cluster_label
        )));
    }
    let body = parsed.body.trim().to_string();
    if body.is_empty() {
        return Err(CompileError::Validation(format!(
            "invalid stage-2 response for '{}': empty body",
            cluster_label
        )));
    }

    // Membership from the validated cluster — not from the model.
    let slug = crate::domain::Slug::generate(&title, "").to_string();

    Ok(ValidatedBranch {
        slug,
        title,
        body,
        leaves: leaf_files.to_vec(),
    })
}

/// Wrap stage-2 results into a CompilePlan that can feed into the existing
/// execute path.
pub(in crate::cli::compile) fn plan_from_stage2(
    updated: Vec<ValidatedBranch>,
    new: Vec<ValidatedBranch>,
) -> CompilePlan {
    let mut branches = updated;
    branches.extend(new);
    CompilePlan { branches }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stage2_response_valid_constructs_branch() {
        let response = Ok(r#"{"title": "Concept Name", "body": "Synthesized body."}"#.to_string());
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let branch = parse_stage2_response(&response, "test-cluster", &leaf_files)
            .expect("valid response should parse");
        assert_eq!(branch.title, "Concept Name");
        assert_eq!(branch.body, "Synthesized body.");
        // Membership comes from cluster, not from the response.
        assert_eq!(branch.leaves, leaf_files);
    }

    #[test]
    fn parse_stage2_response_rejects_malformed_json() {
        let response = Ok("not json".to_string());
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let result = parse_stage2_response(&response, "test-cluster", &leaf_files);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid stage-2"),
            "expected parse error, got: {}",
            err
        );
    }

    #[test]
    fn parse_stage2_response_rejects_empty_title() {
        let response = Ok(r#"{"title": "  ", "body": "Some body."}"#.to_string());
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let result = parse_stage2_response(&response, "test-cluster", &leaf_files);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty title"),
            "expected empty title error, got: {}",
            err
        );
    }

    #[test]
    fn parse_stage2_response_rejects_empty_body() {
        let response = Ok(r#"{"title": "Title", "body": "  "}"#.to_string());
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let result = parse_stage2_response(&response, "test-cluster", &leaf_files);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty body"),
            "expected empty body error, got: {}",
            err
        );
    }

    #[test]
    fn parse_stage2_response_membership_from_cluster_not_response() {
        // Response includes "leaves" field but it is ignored (deny_unknown_fields
        // would reject it — so this tests the schema rejects extra fields).
        let response =
            Ok(r#"{"title": "Concept", "body": "Body.", "leaves": ["hacked.md"]}"#.to_string());
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let result = parse_stage2_response(&response, "test-cluster", &leaf_files);
        // deny_unknown_fields should reject the extra "leaves" field.
        assert!(result.is_err(), "should reject unknown fields");
    }

    #[test]
    fn parse_stage2_response_llm_error_propagates() {
        let response: Result<String, CompileError> = Err(CompileError::Truncated);
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let result = parse_stage2_response(&response, "test-cluster", &leaf_files);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("truncated") || err.contains("LLM call failed"),
            "expected LLM error propagation, got: {}",
            err
        );
    }
}
