// ── two-stage compile: cluster prompt constants and builders ─────────────────

use std::collections::HashSet;

use crate::cli::compile::plan::LoadedLeaf;
use crate::domain::state::TreeState;
use crate::engine::config::SeededConfig;

use super::discovery::{compute_candidate_clusters, CandidateCluster};

// ── prompt constants ─────────────────────────────────────────────────────────

pub(in crate::cli::compile) const CLUSTER_SYSTEM_PROMPT: &str = "\
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

pub(in crate::cli::compile) const INCREMENTAL_CLUSTER_SYSTEM_PROMPT: &str = "\
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

// ── prompts ──────────────────────────────────────────────────────────────────

/// Build the cluster user message for Full mode.
pub(in crate::cli::compile) fn build_cluster_user_message(leaves: &[LoadedLeaf]) -> String {
    let leaf_refs: Vec<&LoadedLeaf> = leaves.iter().collect();
    let candidates = compute_candidate_clusters(&leaf_refs);
    build_cluster_message(&leaf_refs, &candidates, false)
}

/// Build the cluster user message for Incremental mode.
pub(in crate::cli::compile) fn build_incremental_cluster_user_message(
    cfg: &SeededConfig,
    state: &TreeState,
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
    if !state.branches.is_empty() {
        msg.push_str("<existing_branches>\n");
        let tree = cfg.tree();
        for branch in &state.branches {
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
