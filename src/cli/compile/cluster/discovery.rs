// ── two-stage compile: deterministic cluster pre-pass ────────────────────────

use std::collections::HashSet;

use crate::cli::compile::plan::LoadedLeaf;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a LoadedLeaf with a body that creates similarity with
    /// other leaves sharing the same keyword.
    fn leaf_with_body(slug: &str, title: &str, body: &str) -> LoadedLeaf {
        LoadedLeaf {
            slug: slug.to_string(),
            filename: format!("{}.md", slug),
            title: title.to_string(),
            summary: None,
            body: body.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn compute_candidate_clusters_groups_similar_leaves() {
        let leaves: Vec<LoadedLeaf> = vec![
            leaf_with_body(
                "rust-1",
                "Rust Ownership",
                "rust borrow checker memory safety ownership",
            ),
            leaf_with_body(
                "rust-2",
                "Rust Traits",
                "rust trait system generics type safety abstraction",
            ),
            leaf_with_body(
                "python-1",
                "Python Decorators",
                "python decorator function wrapper metaprogramming",
            ),
            leaf_with_body(
                "python-2",
                "Python Generators",
                "python generator yield iterator lazy evaluation",
            ),
        ];
        let leaf_refs: Vec<&LoadedLeaf> = leaves.iter().collect();
        let clusters = compute_candidate_clusters(&leaf_refs);

        // Expect at least one cluster: rust-1 and rust-2 share "rust".
        // python-1 and python-2 share "python".
        assert!(!clusters.is_empty(), "should find at least one cluster");

        // Verify that leaves sharing keywords end up in the same cluster.
        let has_rust_cluster = clusters
            .iter()
            .any(|c| c.leaf_indices.contains(&0) && c.leaf_indices.contains(&1));
        let has_python_cluster = clusters
            .iter()
            .any(|c| c.leaf_indices.contains(&2) && c.leaf_indices.contains(&3));
        assert!(has_rust_cluster, "rust leaves should cluster together");
        assert!(has_python_cluster, "python leaves should cluster together");
    }

    #[test]
    fn compute_candidate_clusters_empty_for_single_leaf() {
        let leaves: Vec<LoadedLeaf> = vec![leaf_with_body("only", "Only", "single leaf body")];
        let leaf_refs: Vec<&LoadedLeaf> = leaves.iter().collect();
        let clusters = compute_candidate_clusters(&leaf_refs);
        assert!(clusters.is_empty());
    }

    #[test]
    fn compute_candidate_clusters_no_clusters_for_unrelated() {
        let leaves: Vec<LoadedLeaf> = vec![
            leaf_with_body("a", "Art", "painting color canvas brush art"),
            leaf_with_body("b", "Bridge", "concrete steel engineering bridge"),
            leaf_with_body("c", "Cooking", "recipe kitchen food cooking"),
        ];
        let leaf_refs: Vec<&LoadedLeaf> = leaves.iter().collect();
        let clusters = compute_candidate_clusters(&leaf_refs);
        // Zero shared terms means no clusters.
        assert!(clusters.is_empty());
    }
}
