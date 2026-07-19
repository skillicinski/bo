use crate::cli::compile::plan::LoadedLeaf;
use crate::domain::state::{TreeMetadata, TreeState};
use crate::domain::{Branch, Leaf, Title, Url};
use crate::domain::{Slug, Timestamp};

use super::validation::{
    validate_clusters, validate_incremental_clusters, BranchAssignment, ClusterAssignment,
    ClusterResponse, IncrementalClusterResponse,
};

fn loaded_leaf(slug: &str, title: &str) -> LoadedLeaf {
    LoadedLeaf {
        slug: slug.to_string(),
        filename: format!("{}.md", slug),
        title: title.to_string(),
        summary: None,
        body: format!("body of {}", title),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

fn leaf_record(slug: &str, file: &str, title: &str, collected_at: &str) -> Leaf {
    Leaf {
        slug: Slug::generate(slug, ""),
        file: file.to_string(),
        title: Title::parse(title).ok(),
        url: Url::parse("https://example.com").unwrap(),
        collected_at: Timestamp::parse(collected_at).unwrap(),
        summary: Some("summary text".to_string()),
    }
}

// ── cluster validation (Full mode) ───────────────────────────────────

#[test]
fn validate_clusters_accepts_valid_response() {
    let leaves = vec![
        loaded_leaf("leaf-1", "Rust"),
        loaded_leaf("leaf-2", "Cargo"),
        loaded_leaf("leaf-3", "Python"),
        loaded_leaf("leaf-4", "Pip"),
    ];
    let response = ClusterResponse {
        clusters: vec![
            ClusterAssignment {
                title: "Rust Ecosystem".to_string(),
                leaf_slugs: vec!["leaf-1".to_string(), "leaf-2".to_string()],
            },
            ClusterAssignment {
                title: "Python Ecosystem".to_string(),
                leaf_slugs: vec!["leaf-3".to_string(), "leaf-4".to_string()],
            },
        ],
    };
    let mut warnings = Vec::new();
    let result = validate_clusters(&response, &leaves, &mut warnings);
    assert!(
        result.is_ok(),
        "valid clusters should pass: {:?}",
        result.err()
    );
    let validated = result.unwrap();
    assert_eq!(validated.clusters.len(), 2);
    assert_eq!(validated.clusters[0].title, "Rust Ecosystem");
    assert_eq!(validated.clusters[1].title, "Python Ecosystem");
}

#[test]
fn validate_clusters_rejects_empty_title() {
    let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
    let response = ClusterResponse {
        clusters: vec![ClusterAssignment {
            title: "  ".to_string(),
            leaf_slugs: vec!["a".to_string(), "b".to_string()],
        }],
    };
    let mut warnings = Vec::new();
    let result = validate_clusters(&response, &leaves, &mut warnings);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("empty title"),
        "expected empty title error, got: {}",
        err
    );
}

#[test]
fn validate_clusters_rejects_duplicate_title() {
    let leaves = vec![
        loaded_leaf("a", "A"),
        loaded_leaf("b", "B"),
        loaded_leaf("c", "C"),
        loaded_leaf("d", "D"),
    ];
    let response = ClusterResponse {
        clusters: vec![
            ClusterAssignment {
                title: "Same".to_string(),
                leaf_slugs: vec!["a".to_string(), "b".to_string()],
            },
            ClusterAssignment {
                title: "Same".to_string(),
                leaf_slugs: vec!["c".to_string(), "d".to_string()],
            },
        ],
    };
    let mut warnings = Vec::new();
    let result = validate_clusters(&response, &leaves, &mut warnings);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("duplicate"),
        "expected duplicate error, got: {}",
        err
    );
}

#[test]
fn validate_clusters_rejects_single_leaf_cluster() {
    let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
    let response = ClusterResponse {
        clusters: vec![ClusterAssignment {
            title: "Solo".to_string(),
            leaf_slugs: vec!["a".to_string()],
        }],
    };
    let mut warnings = Vec::new();
    let result = validate_clusters(&response, &leaves, &mut warnings);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("at least 2"),
        "expected min-leaves error, got: {}",
        err
    );
}

#[test]
fn validate_clusters_repairs_unknown_leaf_and_drops_cluster_if_below_2() {
    let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
    let response = ClusterResponse {
        clusters: vec![ClusterAssignment {
            title: "Concept".to_string(),
            leaf_slugs: vec!["a".to_string(), "nonexistent".to_string()],
        }],
    };
    let mut warnings = Vec::new();
    let result = validate_clusters(&response, &leaves, &mut warnings);
    // Unknown leaf ref dropped → cluster has 1 leaf → cluster dropped → no clusters → error.
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("repaired away"),
        "expected repaired-away error, got: {}",
        err
    );
    // Should have warned about the unknown ref and the cluster drop.
    assert!(
        warnings.iter().any(|w| w.contains("unknown leaf")),
        "should warn about unknown leaf"
    );
}

#[test]
fn validate_clusters_repairs_unknown_leaf_keeps_cluster_if_still_valid() {
    let leaves = vec![
        loaded_leaf("a", "A"),
        loaded_leaf("b", "B"),
        loaded_leaf("c", "C"),
    ];
    let response = ClusterResponse {
        clusters: vec![ClusterAssignment {
            title: "Concept".to_string(),
            leaf_slugs: vec!["a".to_string(), "b".to_string(), "nonexistent".to_string()],
        }],
    };
    let mut warnings = Vec::new();
    let result = validate_clusters(&response, &leaves, &mut warnings);
    // Unknown dropped, 2 remain → cluster survives.
    assert!(result.is_ok());
    let validated = result.unwrap();
    assert_eq!(validated.clusters.len(), 1);
    assert_eq!(validated.clusters[0].leaf_files.len(), 2);
    assert!(
        warnings.iter().any(|w| w.contains("unknown leaf")),
        "should warn about unknown leaf"
    );
}

#[test]
fn validate_clusters_repairs_cross_cluster_duplicate() {
    let leaves = vec![
        loaded_leaf("a", "A"),
        loaded_leaf("b", "B"),
        loaded_leaf("c", "C"),
    ];
    let response = ClusterResponse {
        clusters: vec![
            ClusterAssignment {
                title: "One".to_string(),
                leaf_slugs: vec!["a".to_string(), "b".to_string()],
            },
            ClusterAssignment {
                title: "Two".to_string(),
                leaf_slugs: vec!["b".to_string(), "c".to_string()],
            },
        ],
    };
    let mut warnings = Vec::new();
    let result = validate_clusters(&response, &leaves, &mut warnings);
    // "b" kept in first cluster ("One"), dropped from second. Second has only "c" → below 2 → dropped.
    // First cluster survives with a,b.
    assert!(result.is_ok());
    let validated = result.unwrap();
    assert_eq!(validated.clusters.len(), 1);
    assert_eq!(validated.clusters[0].title, "One");
    assert_eq!(validated.clusters[0].leaf_files.len(), 2);
    assert!(
        warnings.iter().any(|w| w.contains("multiple clusters")),
        "should warn about cross-cluster duplicate"
    );
}

// ── incremental cluster validation ───────────────────────────────────

#[test]
fn validate_incremental_clusters_accepts_assignment_and_new_cluster() {
    let existing_branch = Branch {
        slug: Slug::generate("existing-concept", ""),
        file: "branch/existing-concept.md".to_string(),
        title: Title::parse("Existing Concept").unwrap(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        leaves: vec![Slug::generate("old-leaf", "")],
    };
    let state = TreeState {
        tree: TreeMetadata {
            name: "test".to_string(),
            created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2026-01-02T00:00:00Z").unwrap()),
        },
        leaves: vec![
            leaf_record(
                "old-leaf",
                "old-leaf.md",
                "Old Leaf",
                "2026-01-01T00:00:00Z",
            ),
            leaf_record("new-1", "new-1.md", "New One", "2026-01-03T00:00:00Z"),
            leaf_record("new-2", "new-2.md", "New Two", "2026-01-03T00:00:00Z"),
            leaf_record("new-3", "new-3.md", "New Three", "2026-01-03T00:00:00Z"),
            leaf_record("new-4", "new-4.md", "New Four", "2026-01-03T00:00:00Z"),
        ],
        branches: vec![existing_branch],
    };
    let leaves = vec![
        loaded_leaf("new-1", "New One"),
        loaded_leaf("new-2", "New Two"),
        loaded_leaf("new-3", "New Three"),
        loaded_leaf("new-4", "New Four"),
    ];
    let response = IncrementalClusterResponse {
        assignments: vec![BranchAssignment {
            branch_slug: "existing-concept".to_string(),
            leaf_slugs: vec!["new-1".to_string(), "new-2".to_string()],
        }],
        new_clusters: vec![ClusterAssignment {
            title: "Brand New Concept".to_string(),
            leaf_slugs: vec!["new-3".to_string(), "new-4".to_string()],
        }],
    };
    let mut warnings = Vec::new();
    let result = validate_incremental_clusters(&response, &state, &leaves, &mut warnings);
    assert!(
        result.is_ok(),
        "valid incremental clusters should pass: {:?}",
        result.err()
    );
    let validated = result.unwrap();
    assert_eq!(validated.clusters.len(), 2);
    // First cluster is the assignment to existing branch.
    assert!(validated.clusters[0].is_existing_branch());
    assert_eq!(
        validated.clusters[0].existing_branch_slug,
        "existing-concept"
    );
    // Second cluster is the new cluster.
    assert!(!validated.clusters[1].is_existing_branch());
}

#[test]
fn validate_incremental_clusters_repairs_unknown_branch() {
    let state = TreeState {
        tree: TreeMetadata {
            name: "test".to_string(),
            created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2026-01-02T00:00:00Z").unwrap()),
        },
        leaves: vec![
            leaf_record("new-1", "new-1.md", "New One", "2026-01-03T00:00:00Z"),
            leaf_record("new-2", "new-2.md", "New Two", "2026-01-03T00:00:00Z"),
        ],
        branches: vec![],
    };
    let leaves = vec![
        loaded_leaf("new-1", "New One"),
        loaded_leaf("new-2", "New Two"),
    ];
    let response = IncrementalClusterResponse {
        assignments: vec![BranchAssignment {
            branch_slug: "nonexistent".to_string(),
            leaf_slugs: vec!["new-1".to_string(), "new-2".to_string()],
        }],
        new_clusters: vec![],
    };
    let mut warnings = Vec::new();
    let result = validate_incremental_clusters(&response, &state, &leaves, &mut warnings);
    // Unknown branch → assignment dropped. No clusters remain → error.
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("repaired away"),
        "expected repaired-away error, got: {}",
        err
    );
    assert!(
        warnings.iter().any(|w| w.contains("unknown branch")),
        "should warn about unknown branch"
    );
}

#[test]
fn validate_incremental_clusters_rejects_new_cluster_title_collision() {
    let existing_branch = Branch {
        slug: Slug::generate("existing", ""),
        file: "branch/existing.md".to_string(),
        title: Title::parse("Existing").unwrap(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        leaves: vec![Slug::generate("old", "")],
    };
    let state = TreeState {
        tree: TreeMetadata {
            name: "test".to_string(),
            created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2026-01-02T00:00:00Z").unwrap()),
        },
        leaves: vec![
            leaf_record("new-1", "new-1.md", "New One", "2026-01-03T00:00:00Z"),
            leaf_record("new-2", "new-2.md", "New Two", "2026-01-03T00:00:00Z"),
        ],
        branches: vec![existing_branch],
    };
    let leaves = vec![
        loaded_leaf("new-1", "New One"),
        loaded_leaf("new-2", "New Two"),
    ];
    let response = IncrementalClusterResponse {
        assignments: vec![],
        new_clusters: vec![ClusterAssignment {
            title: "Existing".to_string(),
            leaf_slugs: vec!["new-1".to_string(), "new-2".to_string()],
        }],
    };
    let mut warnings = Vec::new();
    let result = validate_incremental_clusters(&response, &state, &leaves, &mut warnings);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("collides"),
        "expected collision error, got: {}",
        err
    );
}
