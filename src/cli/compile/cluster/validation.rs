// ── two-stage compile: cluster response validation ───────────────────────────

use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::cli::compile::validation::{leaf_resolver, LeafLookup};
use crate::cli::compile::CompileError;
use crate::domain::manifest::Manifest;

// ── cluster response schemas ─────────────────────────────────────────────────

/// Deserialized cluster LLM response for Full mode.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::cli::compile) struct ClusterResponse {
    pub(super) clusters: Vec<ClusterAssignment>,
}

/// Deserialized cluster LLM response for Incremental mode.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(in crate::cli::compile) struct IncrementalClusterResponse {
    /// New leaves assigned to existing branches (by branch slug).
    #[serde(default)]
    pub(super) assignments: Vec<BranchAssignment>,
    /// New clusters for genuinely new concepts.
    #[serde(default)]
    pub(super) new_clusters: Vec<ClusterAssignment>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ClusterAssignment {
    pub(super) title: String,
    pub(super) leaf_slugs: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct BranchAssignment {
    pub(super) branch_slug: String,
    pub(super) leaf_slugs: Vec<String>,
}

// ── validated types ──────────────────────────────────────────────────────────

/// Validated cluster output ready for stage 2.
#[derive(Debug)]
pub(in crate::cli::compile) struct ValidatedClusters {
    /// Clusters for Full mode (all new).
    pub(in crate::cli::compile) clusters: Vec<ValidatedCluster>,
}

#[derive(Debug)]
pub(in crate::cli::compile) struct ValidatedCluster {
    pub(super) title: String,
    /// Canonical leaf filenames (resolved via leaf_resolver).
    pub(super) leaf_files: Vec<String>,
    /// When non-empty, this cluster maps to an existing branch (incremental mode).
    /// Stage 2 reads the existing branch body and produces an update.
    pub(super) existing_branch_slug: String,
}

impl ValidatedCluster {
    fn new(title: String, leaf_files: Vec<String>) -> Self {
        Self {
            title,
            leaf_files,
            existing_branch_slug: String::new(),
        }
    }

    pub(super) fn is_existing_branch(&self) -> bool {
        !self.existing_branch_slug.is_empty()
    }
}

// ── validation ───────────────────────────────────────────────────────────────

/// Validate a Full-mode cluster response.
///
/// Repairs (warn, don't reject):
/// - Unknown leaf ref → dropped from cluster; cluster dropped if it falls below 2.
/// - Leaf in multiple clusters → kept in first, dropped from later clusters.
///
/// Hard rejections: empty title, duplicate title, empty leaf reference,
/// every cluster repaired away to nothing.
pub(in crate::cli::compile) fn validate_clusters(
    response: &ClusterResponse,
    loaded_leaves: &[crate::cli::compile::plan::LoadedLeaf],
    warnings: &mut Vec<String>,
) -> Result<ValidatedClusters, CompileError> {
    let lookup = leaf_resolver(loaded_leaves);
    for msg in &lookup.collisions {
        warnings.push(format!("warning: title collision — {}", msg));
    }

    let mut seen_titles: HashSet<String> = HashSet::new();
    let mut assigned_leaves: HashMap<String, String> = HashMap::new(); // filename → cluster title
    let mut validated = Vec::new();

    for (i, cluster) in response.clusters.iter().enumerate() {
        let title = cluster.title.trim().to_string();
        if title.is_empty() {
            return Err(CompileError::Validation(format!(
                "invalid cluster response: cluster #{} has empty title",
                i + 1
            )));
        }
        if !seen_titles.insert(title.to_lowercase()) {
            return Err(CompileError::Validation(format!(
                "invalid cluster response: duplicate cluster title '{}'",
                title
            )));
        }

        let mut leaf_files = Vec::new();
        let mut seen_in_cluster: HashSet<String> = HashSet::new();
        let mut dropped_unknown = false;
        for raw_leaf in &cluster.leaf_slugs {
            let key = raw_leaf.trim().to_string();
            if key.is_empty() {
                return Err(CompileError::Validation(format!(
                    "invalid cluster response: cluster '{}' has an empty leaf reference",
                    title
                )));
            }
            let Some(normalized) = lookup.map.get(&key.to_lowercase()) else {
                dropped_unknown = true;
                warnings.push(format!(
                    "warning: cluster '{}' references unknown leaf '{}' — dropped",
                    title, key
                ));
                continue;
            };
            if seen_in_cluster.insert(normalized.clone()) {
                leaf_files.push(normalized.clone());
            }
        }
        if dropped_unknown {
            // Re-check leaf count: unknown refs were dropped, cluster may have shrunk.
            if leaf_files.len() < 2 {
                warnings.push(format!(
                    "warning: cluster '{}' dropped below 2 leaves after removing unknown refs — cluster dropped",
                    title
                ));
                continue;
            }
        }

        if leaf_files.len() < 2 {
            return Err(CompileError::Validation(format!(
                "invalid cluster response: cluster '{}' has {} leaf; clusters must have at least 2",
                title,
                leaf_files.len()
            )));
        }

        // Cross-cluster duplicate: keep in first cluster, drop from later ones.
        let mut deduped_files = Vec::new();
        for f in leaf_files {
            if let Some(prev_cluster) = assigned_leaves.get(&f) {
                warnings.push(format!(
                    "warning: leaf '{}' assigned to multiple clusters; kept in '{}', dropped from '{}'",
                    f, prev_cluster, title
                ));
            } else {
                assigned_leaves.insert(f.clone(), title.clone());
                deduped_files.push(f);
            }
        }
        if deduped_files.len() < 2 {
            warnings.push(format!(
                "warning: cluster '{}' dropped below 2 leaves after cross-cluster dedup — cluster dropped",
                title
            ));
            continue;
        }

        validated.push(ValidatedCluster::new(title, deduped_files));
    }

    if validated.is_empty() {
        return Err(CompileError::Validation(
            "invalid cluster response: every cluster was repaired away — no valid clusters remain"
                .to_string(),
        ));
    }

    Ok(ValidatedClusters {
        clusters: validated,
    })
}

/// Validate an Incremental-mode cluster response.
///
/// Repairs (warn, don't reject):
/// - Unknown leaf ref → dropped; cluster/assignment dropped if it falls below minimum.
/// - Leaf in multiple clusters → kept in first, dropped from later.
/// - Assignment to unknown branch slug → assignment dropped.
///
/// Hard rejections: empty/duplicate cluster titles, title collision with
/// existing branch, empty leaf reference, every cluster repaired away.
pub(in crate::cli::compile) fn validate_incremental_clusters(
    response: &IncrementalClusterResponse,
    manifest: &Manifest,
    loaded_leaves: &[crate::cli::compile::plan::LoadedLeaf],
    warnings: &mut Vec<String>,
) -> Result<ValidatedClusters, CompileError> {
    let lookup = leaf_resolver(loaded_leaves);
    for msg in &lookup.collisions {
        warnings.push(format!("warning: title collision — {}", msg));
    }

    let existing_titles_lower: HashSet<String> = manifest
        .branches
        .iter()
        .map(|b| b.title.as_str().to_lowercase())
        .collect();

    let mut assigned_leaves: HashMap<String, String> = HashMap::new(); // filename → cluster title
    let mut seen_assignment_slugs: HashSet<String> = HashSet::new();
    let mut seen_new_titles: HashSet<String> = HashSet::new();
    let mut validated = Vec::new();

    // Repair-resolve leaf refs (shared between assignments and new clusters).
    // Returns (leaf_files, had_unknown_drops).
    let resolve_leaves = |raw_leaves: &[String],
                          cluster_label: &str,
                          lookup: &LeafLookup,
                          warnings: &mut Vec<String>|
     -> (Vec<String>, bool) {
        let mut files = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut dropped = false;
        for raw_leaf in raw_leaves {
            let key = raw_leaf.trim().to_string();
            if key.is_empty() {
                continue;
            }
            let Some(normalized) = lookup.map.get(&key.to_lowercase()) else {
                dropped = true;
                warnings.push(format!(
                    "warning: {} references unknown leaf '{}' — dropped",
                    cluster_label, key
                ));
                continue;
            };
            if seen.insert(normalized.clone()) {
                files.push(normalized.clone());
            }
        }
        (files, dropped)
    };

    // Cross-cluster dedup helper.
    let cross_dedup = |files: Vec<String>,
                       title: &str,
                       assigned_leaves: &mut HashMap<String, String>,
                       warnings: &mut Vec<String>|
     -> Vec<String> {
        let mut deduped = Vec::new();
        for f in files {
            if let Some(prev) = assigned_leaves.get(&f) {
                warnings.push(format!(
                        "warning: leaf '{}' assigned to multiple clusters; kept in '{}', dropped from '{}'",
                        f, prev, title
                    ));
            } else {
                assigned_leaves.insert(f.clone(), title.to_string());
                deduped.push(f);
            }
        }
        deduped
    };

    // Validate assignments (existing branches).
    for assignment in &response.assignments {
        // Unknown branch slug → drop assignment, warn.
        let Some(branch) = manifest.branch_by_slug_str(&assignment.branch_slug) else {
            warnings.push(format!(
                "warning: assignment references unknown branch '{}' — assignment dropped",
                assignment.branch_slug
            ));
            continue;
        };

        if !seen_assignment_slugs.insert(assignment.branch_slug.clone()) {
            return Err(CompileError::Validation(format!(
                "invalid cluster response: duplicate assignment to branch '{}'",
                assignment.branch_slug
            )));
        }

        let label = format!("assignment to '{}'", assignment.branch_slug);
        let (leaf_files, had_unknown) =
            resolve_leaves(&assignment.leaf_slugs, &label, &lookup, warnings);
        if leaf_files.is_empty() {
            if had_unknown {
                warnings.push(format!(
                    "warning: assignment to '{}' has no valid leaves after removing unknown refs — assignment dropped",
                    assignment.branch_slug
                ));
            }
            continue;
        }

        // Cross-cluster dedup.
        let branch_title = branch.title.as_str().to_string();
        let deduped = cross_dedup(leaf_files, &branch_title, &mut assigned_leaves, warnings);
        if deduped.is_empty() {
            warnings.push(format!(
                "warning: assignment to '{}' has no leaves after dedup — assignment dropped",
                assignment.branch_slug
            ));
            continue;
        }

        validated.push(ValidatedCluster {
            title: branch_title,
            leaf_files: deduped,
            existing_branch_slug: branch.slug.as_str().to_string(),
        });
    }

    // Validate new clusters.
    for (i, cluster) in response.new_clusters.iter().enumerate() {
        let title = cluster.title.trim().to_string();
        if title.is_empty() {
            return Err(CompileError::Validation(format!(
                "invalid cluster response: new cluster #{} has empty title",
                i + 1
            )));
        }
        let title_lower = title.to_lowercase();
        if !seen_new_titles.insert(title_lower.clone()) {
            return Err(CompileError::Validation(format!(
                "invalid cluster response: duplicate new cluster title '{}'",
                title
            )));
        }
        if existing_titles_lower.contains(&title_lower) {
            return Err(CompileError::Validation(format!(
                "invalid cluster response: new cluster title '{}' collides with existing branch",
                title
            )));
        }

        let label = format!("new cluster '{}'", title);
        let (leaf_files, had_unknown) =
            resolve_leaves(&cluster.leaf_slugs, &label, &lookup, warnings);

        // Check for empty leaf refs (hard reject — LLM output garbage).
        if cluster.leaf_slugs.iter().any(|s| s.trim().is_empty()) {
            return Err(CompileError::Validation(format!(
                "invalid cluster response: new cluster '{}' has an empty leaf reference",
                title
            )));
        }

        if had_unknown && leaf_files.len() < 2 {
            warnings.push(format!(
                "warning: new cluster '{}' dropped below 2 leaves after removing unknown refs — cluster dropped",
                title
            ));
            continue;
        }

        if leaf_files.len() < 2 {
            return Err(CompileError::Validation(format!(
                "invalid cluster response: new cluster '{}' has {} leaf; clusters must have at least 2",
                title,
                leaf_files.len()
            )));
        }

        let deduped = cross_dedup(leaf_files, &title, &mut assigned_leaves, warnings);
        if deduped.len() < 2 {
            warnings.push(format!(
                "warning: new cluster '{}' dropped below 2 leaves after dedup — cluster dropped",
                title
            ));
            continue;
        }

        validated.push(ValidatedCluster::new(title, deduped));
    }

    if validated.is_empty() {
        return Err(CompileError::Validation(
            "invalid cluster response: every cluster was repaired away — no valid clusters remain"
                .to_string(),
        ));
    }

    Ok(ValidatedClusters {
        clusters: validated,
    })
}
