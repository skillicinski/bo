// ── compile response parsing and validation ───────────────────────────────────

use std::collections::{HashMap, HashSet};

use serde::Deserialize;

use crate::domain::slug;
use crate::engine::config::SeededConfig;

use super::plan::select_new_leaf_slugs;
use super::{CompileError, MAX_COMPILED_BODY_BYTES_MIN, MAX_COMPILED_BODY_BYTES_PER_INPUT_BYTE};

// ── types ─────────────────────────────────────────────────────────────────────

/// Deserialized LLM response.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompileResponse {
    pub(super) branches: Vec<RawBranch>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawBranch {
    pub(super) title: String,
    pub(super) body: String,
    pub(super) leaves: Vec<String>,
}

/// Deserialized incremental LLM response.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct IncrementalCompileResponse {
    pub(super) updated_branches: Vec<RawUpdatedBranch>,
    pub(super) new_branches: Vec<RawBranch>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RawUpdatedBranch {
    pub(super) slug: String,
    pub(super) title: String,
    pub(super) body: String,
    pub(super) leaves: Vec<String>,
}

/// Validated compile plan ready for execution.
#[derive(Debug)]
pub(super) struct CompilePlan {
    pub(super) branches: Vec<ValidatedBranch>,
}

#[derive(Debug)]
pub(super) struct ValidatedBranch {
    pub(super) slug: String,
    pub(super) title: String,
    pub(super) body: String,
    pub(super) leaves: Vec<String>,
}

// ── functions ─────────────────────────────────────────────────────────────────

pub(super) fn valid_leaf_reference_map(valid_filenames: &HashSet<String>) -> HashMap<&str, String> {
    let mut refs = HashMap::new();
    for filename in valid_filenames {
        refs.insert(filename.as_str(), filename.clone());
        if let Some(stem) = filename.strip_suffix(".md") {
            refs.insert(stem, filename.clone());
        }
    }
    refs
}

pub(super) fn parse_and_validate_with_input_size(
    response: &str,
    valid_filenames: &HashSet<String>,
    input_body_bytes: usize,
) -> Result<CompilePlan, CompileError> {
    let parsed: CompileResponse = serde_json::from_str(response)
        .map_err(|e| validation_error(format!("invalid compile response shape: {}", e)))?;

    // Empty branches is valid — means no cross-cutting concepts found.
    if parsed.branches.is_empty() {
        return Ok(CompilePlan {
            branches: Vec::new(),
        });
    }

    let mut validated_branches: Vec<ValidatedBranch> = Vec::new();
    let mut seen_slugs: HashSet<String> = HashSet::new();
    let valid_leaf_refs = valid_leaf_reference_map(valid_filenames);

    for (index, raw) in parsed.branches.into_iter().enumerate() {
        let branch_number = index + 1;
        let title = raw.title.trim().to_string();
        if title.is_empty() {
            return Err(validation_error(format!(
                "invalid compile response: branch #{} has empty title",
                branch_number
            )));
        }
        if raw.body.trim().is_empty() {
            return Err(validation_error(format!(
                "invalid compile response: branch '{}' has empty body",
                title
            )));
        }

        // Generate slug and check uniqueness post-slugification.
        let branch_slug = slug::slugify(&title, "");
        if branch_slug.is_empty() {
            return Err(validation_error(format!(
                "invalid compile response: branch '{}' title produces empty file slug",
                title
            )));
        }
        if seen_slugs.contains(&branch_slug) {
            return Err(validation_error(format!(
                "invalid compile response: duplicate branch slug '{}' (from title '{}') — titles must be distinct",
                branch_slug, title
            )));
        }
        seen_slugs.insert(branch_slug.clone());

        // Validate and deduplicate leaves.
        let mut branch_leaves: Vec<String> = Vec::new();
        let mut seen_leaves: HashSet<String> = HashSet::new();
        for leaf_file in &raw.leaves {
            if leaf_file.trim().is_empty() {
                return Err(validation_error(format!(
                    "invalid compile response: branch '{}' contains an empty leaf reference",
                    title
                )));
            }
            let Some(normalized_leaf_file) = valid_leaf_refs.get(leaf_file.as_str()) else {
                return Err(validation_error(format!(
                    "invalid compile response: branch '{}' references unknown leaf '{}'",
                    title, leaf_file
                )));
            };
            if seen_leaves.insert(normalized_leaf_file.clone()) {
                branch_leaves.push(normalized_leaf_file.clone());
            }
        }

        if branch_leaves.len() < 2 {
            return Err(validation_error(format!(
                "invalid compile response: branch '{}' references {} leaf; branches must reference at least 2 leaves",
                title,
                branch_leaves.len()
            )));
        }

        validated_branches.push(ValidatedBranch {
            slug: branch_slug,
            title,
            body: raw.body,
            leaves: branch_leaves,
        });
    }

    let output_body_bytes = validated_branches
        .iter()
        .map(|branch| branch.body.len())
        .fold(0usize, usize::saturating_add);
    validate_compiled_body_size(input_body_bytes, output_body_bytes)?;

    Ok(CompilePlan {
        branches: validated_branches,
    })
}

pub(super) fn parse_and_validate_incremental_with_input_size(
    response: &str,
    cfg: &SeededConfig,
    valid_filenames: &HashSet<String>,
    input_body_bytes: usize,
) -> Result<CompilePlan, CompileError> {
    let parsed: IncrementalCompileResponse = serde_json::from_str(response).map_err(|e| {
        validation_error(format!("invalid incremental compile response shape: {}", e))
    })?;
    let tree = cfg.tree();
    let manifest = crate::domain::manifest::read(&tree.manifest_path())
        .map_err(|e| CompileError::Io(format!("failed to read manifest: {}", e)))?;
    let new_leaf_slugs_vec = select_new_leaf_slugs(&manifest)?;
    let new_leaf_slugs: HashSet<String> = new_leaf_slugs_vec.into_iter().collect();
    let valid_leaf_refs = valid_leaf_reference_map(valid_filenames);
    let mut seen_branch_slugs = HashSet::new();
    let mut seen_updated_branch_slugs = HashSet::new();
    let mut validated_branches = Vec::new();

    for raw in parsed.updated_branches {
        let existing = manifest.branch_by_slug_str(&raw.slug).ok_or_else(|| {
            validation_error(format!(
                "invalid incremental compile response: update references unknown branch '{}'",
                raw.slug
            ))
        })?;
        if raw.title.trim() != existing.title.as_str() {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' changed title",
                raw.slug
            )));
        }
        let leaves = normalize_incremental_leaf_refs(&raw.title, &raw.leaves, &valid_leaf_refs)?;
        let leaf_slugs: HashSet<String> = leaves
            .iter()
            .map(|leaf| leaf.strip_suffix(".md").unwrap_or(leaf).to_string())
            .collect();
        // Ensure existing leaves are preserved (no drops)
        for existing_leaf in &existing.leaves {
            if !leaf_slugs.contains(existing_leaf.as_str()) {
                return Err(validation_error(format!(
                    "invalid incremental compile response: branch '{}' dropped existing leaf '{}'",
                    raw.slug, existing_leaf
                )));
            }
        }
        // Updated branch must add at least one new leaf
        if !leaf_slugs.iter().any(|slug| new_leaf_slugs.contains(slug)) {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' update adds no newly processed leaf",
                raw.slug
            )));
        }
        if leaves.len() < 2 {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' references {} leaf; branches must reference at least 2 leaves",
                raw.title,
                leaves.len()
            )));
        }
        if !seen_updated_branch_slugs.insert(raw.slug.clone())
            || !seen_branch_slugs.insert(raw.slug.clone())
        {
            return Err(validation_error(format!(
                "invalid incremental compile response: duplicate branch slug '{}'",
                raw.slug
            )));
        }
        validated_branches.push(ValidatedBranch {
            slug: raw.slug,
            title: raw.title,
            body: raw.body,
            leaves,
        });
    }

    for raw in parsed.new_branches {
        let title = raw.title.trim().to_string();
        if title.is_empty() {
            return Err(validation_error(
                "invalid incremental compile response: new branch has empty title",
            ));
        }
        if raw.body.trim().is_empty() {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' has empty body",
                title
            )));
        }
        let branch_slug = slug::slugify(&title, "");
        if manifest.branch_by_slug_str(&branch_slug).is_some()
            || !seen_branch_slugs.insert(branch_slug.clone())
        {
            return Err(validation_error(format!(
                "invalid incremental compile response: duplicate branch slug '{}'",
                branch_slug
            )));
        }
        let leaves = normalize_incremental_leaf_refs(&title, &raw.leaves, &valid_leaf_refs)?;
        if leaves.len() < 2 {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' references {} leaf; branches must reference at least 2 leaves",
                title,
                leaves.len()
            )));
        }
        if !leaves.iter().any(|leaf| {
            let slug = leaf.strip_suffix(".md").unwrap_or(leaf);
            new_leaf_slugs.contains(slug)
        }) {
            // LLM tried to reorganize existing content without integrating a new leaf.
            // Silently drop rather than fail — this is a common model misbehaviour.
            continue;
        }
        validated_branches.push(ValidatedBranch {
            slug: branch_slug,
            title,
            body: raw.body,
            leaves,
        });
    }

    let output_body_bytes = validated_branches
        .iter()
        .map(|branch| branch.body.len())
        .fold(0usize, usize::saturating_add);
    validate_compiled_body_size(input_body_bytes, output_body_bytes)?;

    Ok(CompilePlan {
        branches: validated_branches,
    })
}

pub(super) fn normalize_incremental_leaf_refs(
    branch_title: &str,
    raw_leaves: &[String],
    valid_leaf_refs: &HashMap<&str, String>,
) -> Result<Vec<String>, CompileError> {
    let mut leaves = Vec::new();
    let mut seen = HashSet::new();
    for raw_leaf in raw_leaves {
        let Some(normalized) = valid_leaf_refs.get(raw_leaf.as_str()) else {
            return Err(validation_error(format!(
                "invalid incremental compile response: branch '{}' references unknown leaf '{}'",
                branch_title, raw_leaf
            )));
        };
        if seen.insert(normalized.clone()) {
            leaves.push(normalized.clone());
        }
    }
    Ok(leaves)
}

pub(super) fn validate_compiled_body_size(
    input_body_bytes: usize,
    output_body_bytes: usize,
) -> Result<(), CompileError> {
    let limit = input_body_bytes
        .saturating_mul(MAX_COMPILED_BODY_BYTES_PER_INPUT_BYTE)
        .max(MAX_COMPILED_BODY_BYTES_MIN);

    if output_body_bytes > limit {
        return Err(validation_error(format!(
            "invalid compile response: branch bodies total {} bytes, exceeding {} byte limit for {} bytes of input",
            output_body_bytes, limit, input_body_bytes
        )));
    }

    Ok(())
}

pub(super) fn validation_error(message: impl Into<String>) -> CompileError {
    CompileError::Validation(message.into())
}
