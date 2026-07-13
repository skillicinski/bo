// ── two-stage compile: cluster pass ──────────────────────────────────────────
//
// Stage 1 (cluster): deterministic pre-pass over leaf bodies to produce
// candidate groupings, then one LLM call over titles+summaries that adjudicates
// and names clusters. Stage 2 (synthesize) reuses the existing per-branch
// parse/validate/execute path — this module handles both stages.

use std::collections::{HashMap, HashSet};

use schemars::JsonSchema;
use serde::Deserialize;

use crate::domain::manifest::Manifest;
use crate::engine::config::SeededConfig;
use crate::engine::schema::inline_schema_for;

use super::plan::LoadedLeaf;
use super::prompt::COMPILE_SYSTEM_PROMPT;
use super::validation::{leaf_resolver, CompilePlan, ValidatedBranch};
use super::CompileError;

// ── cluster response schemas ─────────────────────────────────────────────────

/// Deserialized cluster LLM response for Full mode.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct ClusterResponse {
    pub(super) clusters: Vec<ClusterAssignment>,
}

/// Deserialized cluster LLM response for Incremental mode.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(super) struct IncrementalClusterResponse {
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
pub(super) struct ValidatedClusters {
    /// Clusters for Full mode (all new).
    pub(super) clusters: Vec<ValidatedCluster>,
}

#[derive(Debug)]
pub(super) struct ValidatedCluster {
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

// ── prompt constants ─────────────────────────────────────────────────────────

pub(super) const CLUSTER_SYSTEM_PROMPT: &str = "\
You are clustering documents for a knowledge tree compilation. Below are document \
titles and summaries.

Candidate groupings (computed from term similarity) are provided as hints — you \
can adjust, split, or merge them.

## Rules

- Assign each document to at most one cluster.
- Each cluster must contain at least 2 documents. If a document fits no cluster, \
  leave it unassigned.
- Cluster titles must be distinct and specific — avoid generic names like \
  \"Technology\" or \"General\".
- Prefer splitting over forcing unrelated documents into a cluster.
";

pub(super) const INCREMENTAL_CLUSTER_SYSTEM_PROMPT: &str = "\
You are clustering new documents into an existing knowledge tree. Below are new \
document titles and summaries, plus existing branch titles and summaries.

Candidate groupings (computed from term similarity) are provided as hints — you \
can adjust, split, or merge them.

## Rules

- Assign each new document to at most one cluster or existing branch.
- Each cluster must contain at least 2 documents. If a document fits no cluster, \
  leave it unassigned.
- Use `assignments` to map new documents to existing branches (by branch slug) \
  when they extend an existing concept. A single-document assignment is valid \
  (it's extending, not creating).
- Use `new_clusters` for genuinely new concepts not covered by any existing \
  branch. Each new cluster needs a distinct, specific title and at least 2 \
  documents.
- Do not assign documents to a branch they don't clearly belong to.
- Existing branch slugs are exact — copy them verbatim.
";

// ── deterministic pre-pass ───────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub(super) struct CandidateCluster {
    pub(super) leaf_indices: Vec<usize>,
}

/// Compute candidate clusters from leaf bodies using term-density similarity.
///
/// 1. Tokenize each leaf body (via retrieval's `tokenize`).
/// 2. Compute pairwise Jaccard similarity of token sets.
/// 3. Build an adjacency graph (similarity > threshold).
/// 4. Find connected components as candidate clusters.
///
/// Returns clusters of size ≥ 2, sorted largest first.
pub(super) fn compute_candidate_clusters(leaves: &[&LoadedLeaf]) -> Vec<CandidateCluster> {
    if leaves.len() < 2 {
        return Vec::new();
    }

    let token_sets: Vec<HashSet<String>> = leaves
        .iter()
        .map(|leaf| {
            let searchable = format!(
                "{} {} {}",
                leaf.title,
                leaf.summary.as_deref().unwrap_or(""),
                leaf.body
            )
            .to_lowercase();
            crate::engine::retrieval::tokenize(&searchable)
                .into_iter()
                .collect::<HashSet<_>>()
        })
        .collect();

    let n = leaves.len();

    // ponytail: 0.03 threshold tuned against the 232-leaf tommys corpus;
    // re-tune if clusters are too large/small for other corpora.
    const JACCARD_THRESHOLD: f64 = 0.03;
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        for j in (i + 1)..n {
            let intersection = token_sets[i].intersection(&token_sets[j]).count();
            let union = token_sets[i].union(&token_sets[j]).count();
            if union == 0 {
                continue;
            }
            let jaccard = intersection as f64 / union as f64;
            if jaccard > JACCARD_THRESHOLD {
                adj[i].push(j);
                adj[j].push(i);
            }
        }
    }

    // Connected components via DFS.
    let mut visited = vec![false; n];
    let mut components: Vec<Vec<usize>> = Vec::new();
    for start in 0..n {
        if visited[start] {
            continue;
        }
        let mut stack = vec![start];
        let mut component = Vec::new();
        while let Some(v) = stack.pop() {
            if visited[v] {
                continue;
            }
            visited[v] = true;
            component.push(v);
            for &neighbor in &adj[v] {
                if !visited[neighbor] {
                    stack.push(neighbor);
                }
            }
        }
        if component.len() >= 2 {
            component.sort_unstable();
            components.push(component);
        }
    }

    components.sort_by_key(|c| std::cmp::Reverse(c.len()));

    components
        .into_iter()
        .map(|indices| CandidateCluster {
            leaf_indices: indices,
        })
        .collect()
}

// ── prompts ──────────────────────────────────────────────────────────────────

/// Build the cluster user message for Full mode.
pub(super) fn build_cluster_user_message(leaves: &[LoadedLeaf]) -> String {
    let leaf_refs: Vec<&LoadedLeaf> = leaves.iter().collect();
    let candidates = compute_candidate_clusters(&leaf_refs);
    build_cluster_message(&leaf_refs, &candidates, false)
}

/// Build the cluster user message for Incremental mode.
pub(super) fn build_incremental_cluster_user_message(
    cfg: &SeededConfig,
    manifest: &Manifest,
    leaves: &[LoadedLeaf],
    new_leaf_slugs: &[String],
) -> String {
    let new_leaf_set: HashSet<&str> = new_leaf_slugs.iter().map(String::as_str).collect();
    let new_leaves: Vec<&LoadedLeaf> = leaves
        .iter()
        .filter(|l| new_leaf_set.contains(l.slug.as_str()))
        .collect();

    let candidates = compute_candidate_clusters(&new_leaves);

    let mut msg = String::new();

    // Existing branches as anchors with their body summaries.
    if !manifest.branches.is_empty() {
        msg.push_str("<existing_branches>\n");
        let tree = cfg.tree();
        for branch in &manifest.branches {
            let leaves_str: Vec<&str> = branch.leaves.iter().map(|s| s.as_str()).collect();
            msg.push_str(&format!(
                "<branch slug=\"{}\" title=\"{}\" leaves=\"{}\">\n",
                branch.slug,
                branch.title,
                leaves_str.join(",")
            ));
            if let Ok(content) = std::fs::read_to_string(tree.join(&branch.file)) {
                if let Ok((_, body)) = crate::domain::frontmatter::parse(&content) {
                    let summary = crate::engine::summary::generate_fallback(&body);
                    msg.push_str(&format!("<summary>{}</summary>\n", summary));
                }
            }
            msg.push_str("</branch>\n");
        }
        msg.push_str("</existing_branches>\n\n");
    }

    build_cluster_message(&new_leaves, &candidates, true)
}

fn build_cluster_message(
    leaves: &[&LoadedLeaf],
    candidate_clusters: &[CandidateCluster],
    incremental: bool,
) -> String {
    let mut msg = format!("There are {} documents to cluster.\n\n", leaves.len());

    // Leaf catalogue: titles + summaries only (no full bodies).
    msg.push_str("<leaf_catalogue>\n");
    for leaf in leaves {
        msg.push_str(&format!(
            "<document slug=\"{}\" filename=\"{}\" title=\"{}\">\n",
            leaf.slug, leaf.filename, leaf.title
        ));
        if let Some(summary) = &leaf.summary {
            msg.push_str(&format!("<summary>{}</summary>\n", summary));
        }
        msg.push_str("</document>\n");
    }
    msg.push_str("</leaf_catalogue>\n\n");

    // Candidate cluster hints from deterministic pre-pass.
    if !candidate_clusters.is_empty() {
        msg.push_str("<candidate_clusters>\n");
        msg.push_str("These groupings are computed from term similarity — they are hints, not constraints. Adjust, split, or merge as needed.\n\n");
        for (i, cluster) in candidate_clusters.iter().enumerate() {
            let slugs: Vec<&str> = cluster
                .leaf_indices
                .iter()
                .map(|&idx| leaves[idx].slug.as_str())
                .collect();
            msg.push_str(&format!("  Group {}: {}\n", i + 1, slugs.join(", ")));
        }
        msg.push_str("</candidate_clusters>\n\n");
    }

    if incremental {
        msg.push_str(
            "Output assignments (for existing branches) and new_clusters (for new concepts).\n",
        );
    } else {
        msg.push_str("Output a list of clusters with titles and leaf assignments.\n");
    }

    msg
}

// ── stage 2 synthesize prompt ────────────────────────────────────────────────

/// Build the stage-2 synthesize user message for a new cluster.
pub(super) fn build_synthesize_user_message(cluster_title: &str, leaves: &[&LoadedLeaf]) -> String {
    let mut msg = format!(
        "The following documents are all related to the concept: \"{}\".\n\n\
         Produce a single branch that synthesizes how this concept manifests across \
         these documents. Draw connections, note contrasts, highlight patterns. \
         Reference documents by their slug or filename exactly as provided.\n\n\
         There are {} documents.\n\n",
        cluster_title,
        leaves.len()
    );

    for leaf in leaves {
        msg.push_str(&format!(
            "<document slug=\"{}\" filename=\"{}\" title=\"{}\">\n{}\n</document>\n\n",
            leaf.slug, leaf.filename, leaf.title, leaf.body
        ));
    }

    msg
}

/// Build the stage-2 synthesize-update user message for an existing branch.
pub(super) fn build_synthesize_update_user_message(
    branch_title: &str,
    existing_body: &str,
    new_leaves: &[&LoadedLeaf],
) -> String {
    let mut msg = format!(
        "The following new documents extend the existing concept: \"{}\".\n\n\
         Produce an updated branch body that integrates these new documents with the \
         existing synthesis below. Draw connections, note contrasts, highlight \
         patterns. Reference documents by their slug or filename exactly as provided.\n\n",
        branch_title,
    );

    msg.push_str("<existing_branch_body>\n");
    msg.push_str(existing_body);
    msg.push_str("\n</existing_branch_body>\n\n");

    msg.push_str(&format!(
        "There are {} new documents to integrate.\n\n",
        new_leaves.len()
    ));

    for leaf in new_leaves {
        msg.push_str(&format!(
            "<document slug=\"{}\" filename=\"{}\" title=\"{}\">\n{}\n</document>\n\n",
            leaf.slug, leaf.filename, leaf.title, leaf.body
        ));
    }

    msg
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
pub(super) fn validate_clusters(
    response: &ClusterResponse,
    loaded_leaves: &[LoadedLeaf],
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
pub(super) fn validate_incremental_clusters(
    response: &IncrementalClusterResponse,
    manifest: &Manifest,
    loaded_leaves: &[LoadedLeaf],
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
                          lookup: &super::validation::LeafLookup,
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

// ── stage 2: per-cluster synthesize ──────────────────────────────────────────

/// Run stage 2 for a set of validated clusters in Full mode.
/// Each cluster gets one LLM call with that cluster's full leaf bodies.
///
/// ponytail: sequential calls for v1; parallelize with tokio::spawn if
/// per-cluster latency becomes the bottleneck (independent calls, same provider).
pub(super) fn run_stage2_synthesize(
    provider: &dyn crate::engine::llm::LlmProvider,
    model: &crate::engine::llm::Model,
    clusters: &ValidatedClusters,
    all_leaves: &[LoadedLeaf],
    warnings: &mut Vec<String>,
) -> Result<Vec<ValidatedBranch>, CompileError> {
    let schema =
        serde_json::to_value(inline_schema_for::<super::parse::CompileResponse>()).unwrap();
    let mut all_branches: Vec<ValidatedBranch> = Vec::new();
    let mut seen_slugs: HashSet<String> = HashSet::new();

    for cluster in &clusters.clusters {
        let cluster_leaves: Vec<&LoadedLeaf> = cluster
            .leaf_files
            .iter()
            .filter_map(|f| all_leaves.iter().find(|l| l.filename == *f))
            .collect();

        let user_message = build_synthesize_user_message(&cluster.title, &cluster_leaves);
        let prompt_tokens = super::execute::estimate_compile_prompt_tokens(
            COMPILE_SYSTEM_PROMPT
                .len()
                .saturating_add(user_message.len()),
        );
        super::execute::ensure_compile_context_fits(model, prompt_tokens)?;

        let response = super::execute::call_llm_blocking(provider, model, &user_message, &schema)?;
        let cluster_input_bytes: usize = cluster_leaves.iter().map(|l| l.body.len()).sum();

        let parsed: super::parse::CompileResponse =
            serde_json::from_str(&response).map_err(|e| {
                CompileError::Validation(format!(
                    "invalid stage-2 response for cluster '{}': {}",
                    cluster.title, e
                ))
            })?;

        if parsed.branches.is_empty() {
            warnings.push(format!(
                "warning: cluster '{}' produced no branch — skipping",
                cluster.title
            ));
            continue;
        }

        let cluster_plan = super::validation::validate_full(
            parsed,
            &cluster_leaves
                .iter()
                .map(|l| (*l).clone())
                .collect::<Vec<_>>(),
            cluster_input_bytes,
            warnings,
        )?;

        for branch in cluster_plan.branches {
            let slug = crate::domain::Slug::generate(&branch.title, "").to_string();
            if !seen_slugs.insert(slug.clone()) {
                warnings.push(format!(
                    "warning: cluster '{}' produced branch with duplicate slug '{}' — skipping",
                    cluster.title, slug
                ));
                continue;
            }
            all_branches.push(branch);
        }
    }

    Ok(all_branches)
}

/// Run stage 2 for incremental mode: existing-branch assignments get update
/// calls; new clusters get fresh synthesize calls.
///
/// Returns (updated_branches, new_branches).
///
/// ponytail: sequential calls for v1.
pub(super) fn run_stage2_synthesize_incremental(
    cfg: &SeededConfig,
    provider: &dyn crate::engine::llm::LlmProvider,
    model: &crate::engine::llm::Model,
    clusters: &ValidatedClusters,
    manifest: &Manifest,
    all_leaves: &[LoadedLeaf],
    warnings: &mut Vec<String>,
) -> Result<(Vec<ValidatedBranch>, Vec<ValidatedBranch>), CompileError> {
    let schema =
        serde_json::to_value(inline_schema_for::<super::parse::CompileResponse>()).unwrap();
    let tree = cfg.tree();
    let mut updated_branches: Vec<ValidatedBranch> = Vec::new();
    let mut new_branches: Vec<ValidatedBranch> = Vec::new();
    let mut seen_slugs: HashSet<String> = HashSet::new();

    for cluster in &clusters.clusters {
        let cluster_leaves: Vec<&LoadedLeaf> = cluster
            .leaf_files
            .iter()
            .filter_map(|f| all_leaves.iter().find(|l| l.filename == *f))
            .collect();

        if cluster.is_existing_branch() {
            // Read the existing branch body.
            let existing = manifest
                .branch_by_slug_str(&cluster.existing_branch_slug)
                .ok_or_else(|| {
                    CompileError::Validation(format!(
                        "internal error: existing branch '{}' not found",
                        cluster.existing_branch_slug
                    ))
                })?;

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
            let prompt_tokens = super::execute::estimate_compile_prompt_tokens(
                COMPILE_SYSTEM_PROMPT
                    .len()
                    .saturating_add(user_message.len()),
            );
            super::execute::ensure_compile_context_fits(model, prompt_tokens)?;

            let response =
                super::execute::call_llm_blocking(provider, model, &user_message, &schema)?;

            let parsed: super::parse::CompileResponse =
                serde_json::from_str(&response).map_err(|e| {
                    CompileError::Validation(format!(
                        "invalid stage-2 response for branch '{}': {}",
                        existing.slug, e
                    ))
                })?;

            if parsed.branches.is_empty() {
                warnings.push(format!(
                    "warning: update for branch '{}' produced no output — keeping existing body",
                    existing.slug
                ));
                continue;
            }

            let branch = &parsed.branches[0];
            let mut all_leaf_files: Vec<String> = cluster.leaf_files.clone();
            // Add existing leaf filenames.
            for leaf_slug in &existing.leaves {
                if let Some(leaf) = manifest.leaves.iter().find(|l| l.slug == *leaf_slug) {
                    let f = leaf.file.clone();
                    if !all_leaf_files.contains(&f) {
                        all_leaf_files.push(f);
                    }
                }
            }

            let slug = existing.slug.as_str().to_string();
            if seen_slugs.insert(slug.clone()) {
                updated_branches.push(ValidatedBranch {
                    slug,
                    title: existing.title.as_str().to_string(),
                    body: branch.body.clone(),
                    leaves: all_leaf_files,
                });
            }
        } else {
            // New cluster — same as Full mode.
            let user_message = build_synthesize_user_message(&cluster.title, &cluster_leaves);
            let prompt_tokens = super::execute::estimate_compile_prompt_tokens(
                COMPILE_SYSTEM_PROMPT
                    .len()
                    .saturating_add(user_message.len()),
            );
            super::execute::ensure_compile_context_fits(model, prompt_tokens)?;
            let response =
                super::execute::call_llm_blocking(provider, model, &user_message, &schema)?;
            let cluster_input_bytes: usize = cluster_leaves.iter().map(|l| l.body.len()).sum();

            let parsed: super::parse::CompileResponse =
                serde_json::from_str(&response).map_err(|e| {
                    CompileError::Validation(format!(
                        "invalid stage-2 response for cluster '{}': {}",
                        cluster.title, e
                    ))
                })?;

            if parsed.branches.is_empty() {
                warnings.push(format!(
                    "warning: cluster '{}' produced no branch — skipping",
                    cluster.title
                ));
                continue;
            }

            let cluster_plan = super::validation::validate_full(
                parsed,
                &cluster_leaves
                    .iter()
                    .map(|l| (*l).clone())
                    .collect::<Vec<_>>(),
                cluster_input_bytes,
                warnings,
            )?;

            for branch in cluster_plan.branches {
                let slug = crate::domain::Slug::generate(&branch.title, "").to_string();
                if !seen_slugs.insert(slug.clone()) {
                    warnings.push(format!(
                        "warning: cluster '{}' produced branch with duplicate slug '{}' — skipping",
                        cluster.title, slug
                    ));
                    continue;
                }
                new_branches.push(branch);
            }
        }
    }

    Ok((updated_branches, new_branches))
}

/// Wrap stage-2 results into a CompilePlan that can feed into the existing
/// execute path.
pub(super) fn plan_from_stage2(
    updated: Vec<ValidatedBranch>,
    new: Vec<ValidatedBranch>,
) -> CompilePlan {
    let mut branches = updated;
    branches.extend(new);
    CompilePlan { branches }
}
