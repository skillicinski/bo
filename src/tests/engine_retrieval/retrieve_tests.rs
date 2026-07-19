// retrieve_docs integration tests: the combined leaf+branch ranking pipeline
// (document loading, scoring, diagnostics, top-k truncation, error paths).

use crate::domain::slug::Slug;
use crate::domain::state::{TreeMetadata, TreeState};
use crate::domain::timestamp::Timestamp;
use crate::domain::{Branch, Leaf, Title, Url};
use crate::engine::retrieval::{
    retrieve_docs, validate_citations, DocKind, RetrievalDiagnostics, RetrievalError, RetrievedDoc,
    SynthesisResponse,
};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ── on-disk tree builders (retrieve_docs reads .bo/state.json) ─────

fn make_leaf(
    dir: &Path,
    filename: &str,
    title: &str,
    url: &str,
    summary: Option<&str>,
    body: &str,
) {
    let leaves_dir = dir.join("leaves");
    fs::create_dir_all(&leaves_dir).unwrap();

    let mut content = String::from("---\n");
    content.push_str(&format!("title: \"{}\"\n", title));
    content.push_str(&format!("url: \"{}\"\n", url));
    if let Some(s) = summary {
        content.push_str(&format!("summary: \"{}\"\n", s));
    }
    content.push_str("---\n\n");
    content.push_str(body);

    fs::write(leaves_dir.join(filename), content).unwrap()
}

fn make_state(dir: &Path, entries: &[(&str, &str, &str)]) {
    let leaves: Vec<_> = entries
        .iter()
        .map(|(file, title, url)| {
            let summary = fs::read_to_string(dir.join(file))
                .ok()
                .and_then(|content| crate::domain::frontmatter::parse(&content).ok())
                .and_then(|(mapping, _)| {
                    mapping
                        .get("summary")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                });
            Leaf {
                slug: Slug::generate(&Path::new(file).file_stem().unwrap().to_string_lossy(), ""),
                file: file.to_string(),
                title: Title::parse(title).ok(),
                url: Url::parse(url).unwrap(),
                collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                summary,
            }
        })
        .collect();
    let bo_dir = dir.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    crate::engine::state::write(
        &bo_dir.join("state.json"),
        &TreeState {
            tree: TreeMetadata {
                name: "query".to_string(),
                created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves,
            branches: Vec::new(),
        },
    )
    .unwrap()
}

#[test]
fn retrieve_or_semantics_scores_partial_matches() {
    let dir = TempDir::new().unwrap();
    let tree = dir.path();

    make_leaf(
        tree,
        "ownership.md",
        "Understanding Ownership",
        "https://example.com/ownership",
        Some("Rust ownership and borrowing"),
        "Ownership is a key feature of Rust. It ensures memory safety without a garbage collector.",
    );
    make_leaf(
        tree,
        "lifetimes.md",
        "Lifetimes in Rust",
        "https://example.com/lifetimes",
        Some("How lifetimes work"),
        "Lifetimes ensure references are valid. They are part of Rust's type system.",
    );
    make_leaf(
        tree,
        "cooking.md",
        "Cooking Tips",
        "https://example.com/cooking",
        Some("How to cook pasta"),
        "Boil water and add salt. Cook pasta for 10 minutes.",
    );

    make_state(
        tree,
        &[
            (
                "leaves/ownership.md",
                "Understanding Ownership",
                "https://example.com/ownership",
            ),
            (
                "leaves/lifetimes.md",
                "Lifetimes in Rust",
                "https://example.com/lifetimes",
            ),
            (
                "leaves/cooking.md",
                "Cooking Tips",
                "https://example.com/cooking",
            ),
        ],
    );

    let terms = vec!["rust".to_string(), "ownership".to_string()];
    let results = retrieve_docs(tree, &terms).unwrap();

    // ownership leaf should rank highest (both terms match densely)
    assert_eq!(results[0].slug.as_str(), "ownership");
    // lifetimes should match (contains "rust")
    assert!(results.iter().any(|r| r.slug.as_str() == "lifetimes"));
    // cooking should NOT match
    assert!(!results.iter().any(|r| r.slug.as_str() == "cooking"));
}

#[test]
fn retrieve_empty_tree_returns_error() {
    let dir = TempDir::new().unwrap();
    let tree = dir.path();
    make_state(tree, &[]);

    let err = retrieve_docs(tree, &["rust".to_string()]).unwrap_err();
    assert!(matches!(err, RetrievalError::EmptyTree));
}

#[test]
fn retrieve_no_matches_returns_error() {
    let dir = TempDir::new().unwrap();
    let tree = dir.path();

    make_leaf(
        tree,
        "cooking.md",
        "Cooking Tips",
        "https://example.com/cooking",
        Some("How to cook"),
        "Boil water.",
    );
    make_state(
        tree,
        &[(
            "leaves/cooking.md",
            "Cooking Tips",
            "https://example.com/cooking",
        )],
    );

    let err = retrieve_docs(tree, &["rust".to_string()]).unwrap_err();
    assert!(matches!(err, RetrievalError::NoResults));
}

#[test]
fn retrieve_missing_summary_uses_body_fallback() {
    let dir = TempDir::new().unwrap();
    let tree = dir.path();

    make_leaf(
        tree,
        "nosummary.md",
        "No Summary Leaf",
        "https://example.com/ns",
        None,
        "This leaf has no summary field but has a body about Rust programming.",
    );
    make_state(
        tree,
        &[(
            "leaves/nosummary.md",
            "No Summary Leaf",
            "https://example.com/ns",
        )],
    );

    let terms = vec!["rust".to_string()];
    let results = retrieve_docs(tree, &terms).unwrap();

    assert_eq!(results[0].slug.as_str(), "nosummary");
    // Summary should be the body fallback (body is short, so full body used)
    assert!(results[0].summary.contains("Rust programming"));
}

// Retrieval must reach compiled branches, not just raw leaves — otherwise
// `bo compile`'s synthesized output is invisible at retrieval time.
#[test]
fn retrieve_returns_compiled_branch_when_no_leaf_matches() {
    let dir = TempDir::new().unwrap();
    let tree = dir.path();

    // Two leaves about unrelated topics that do NOT mention the query terms.
    make_leaf(
        tree,
        "cooking.md",
        "Cooking Tips",
        "https://example.com/cooking",
        Some("How to cook"),
        "Boil water. Chop vegetables. Simmer for twenty minutes.",
    );
    make_leaf(
        tree,
        "sports.md",
        "Sports News",
        "https://example.com/sports",
        Some("Match reports"),
        "The team won the final. Goals were scored in each half.",
    );

    // A compiled branch synthesizing the concept the user asks about. Its body
    // mentions the query terms; no individual leaf does.
    fs::create_dir_all(tree.join("branch")).unwrap();
    fs::write(
        tree.join("branch/rust-ownership.md"),
        "---\n\
         title: \"Rust Ownership\"\n\
         created_at: 2025-01-01T00:00:00Z\n\
         updated_at: 2025-01-01T00:00:00Z\n\
         leaves: []\n\
         ---\n\n\
         # Rust Ownership\n\n\
         Rust ownership is the core memory-safety mechanism. The borrow checker \
         enforces ownership rules at compile time.\n",
    )
    .unwrap();

    let state = TreeState {
        tree: TreeMetadata {
            name: "query".to_string(),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2025-01-01T00:00:00Z").unwrap()),
        },
        leaves: vec![
            Leaf {
                slug: Slug::generate("cooking", ""),
                file: "leaves/cooking.md".to_string(),
                title: Some(Title::parse("Cooking Tips").unwrap()),
                url: Url::parse("https://example.com/cooking").unwrap(),
                collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                summary: Some("How to cook".to_string()),
            },
            Leaf {
                slug: Slug::generate("sports", ""),
                file: "leaves/sports.md".to_string(),
                title: Some(Title::parse("Sports News").unwrap()),
                url: Url::parse("https://example.com/sports").unwrap(),
                collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                summary: Some("Match reports".to_string()),
            },
        ],
        branches: vec![Branch {
            slug: Slug::generate("Rust Ownership", ""),
            file: "branch/rust-ownership.md".to_string(),
            title: Title::parse("Rust Ownership").unwrap(),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            updated_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            leaves: Vec::new(),
        }],
    };
    let bo_dir = tree.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    crate::engine::state::write(&bo_dir.join("state.json"), &state).unwrap();

    // Only the branch matches "ownership"; neither leaf does.
    let results = retrieve_docs(tree, &["ownership".to_string()]).unwrap();

    assert_eq!(results.len(), 1, "only the branch should match the query");
    assert_eq!(results[0].slug.as_str(), "rust-ownership");
    assert_eq!(
        results[0].kind,
        DocKind::Branch,
        "the matching document must be the compiled branch"
    );

    // The branch must be a citable source after synthesis.
    let retrieved = vec![RetrievedDoc {
        kind: DocKind::Branch,
        slug: "rust-ownership".to_string(),
        title: "Rust Ownership".to_string(),
        url: String::new(),
        file: "branch/rust-ownership.md".to_string(),
        summary: "summary".to_string(),
        body: "body".to_string(),
        score: 1.0,
        diagnostics: RetrievalDiagnostics::default(),
    }];
    let (_answer, citations) = validate_citations(
        SynthesisResponse {
            answer: "See [[rust-ownership]] for the synthesis.".to_string(),
            cited_slugs: vec!["rust-ownership".to_string()],
        },
        &retrieved,
    );
    assert_eq!(citations.len(), 1);
    assert_eq!(citations[0].slug.as_str(), "rust-ownership");
}

#[test]
fn scorer_leaf_and_branch_equal_scores_for_identical_content() {
    let dir = TempDir::new().unwrap();
    let tree = dir.path();

    // Identical content as leaf and branch
    make_leaf(
        tree,
        "topic.md",
        "Topic",
        "https://x.com/topic",
        None,
        "lorem ipsum dolor sit amet",
    );
    fs::create_dir_all(tree.join("branch")).unwrap();
    fs::write(
        tree.join("branch/topic.md"),
        "---\ntitle: \"Topic\"\ncreated_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\nleaves: []\n---\n\nlorem ipsum dolor sit amet\n",
    )
    .unwrap();

    let state = TreeState {
        tree: TreeMetadata {
            name: "test".to_string(),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2025-01-01T00:00:00Z").unwrap()),
        },
        leaves: vec![Leaf {
            slug: Slug::generate("topic", ""),
            file: "leaves/topic.md".to_string(),
            title: Some(Title::parse("Topic").unwrap()),
            url: Url::parse("https://x.com/topic").unwrap(),
            collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            summary: None,
        }],
        branches: vec![Branch {
            slug: Slug::generate("Topic", ""),
            file: "branch/topic.md".to_string(),
            title: Title::parse("Topic").unwrap(),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            updated_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            leaves: Vec::new(),
        }],
    };
    let bo_dir = tree.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    crate::engine::state::write(&bo_dir.join("state.json"), &state).unwrap();

    let terms = vec!["lorem".to_string()];
    let results = retrieve_docs(tree, &terms).unwrap();

    // Both leaf and branch should match, with equal scores (identical content
    // in a single combined corpus → same IDF, same token count, same hits).
    assert_eq!(results.len(), 2);
    let leaf_score = results
        .iter()
        .find(|r| r.kind == DocKind::Leaf)
        .unwrap()
        .score;
    let branch_score = results
        .iter()
        .find(|r| r.kind == DocKind::Branch)
        .unwrap()
        .score;
    assert!(
        (leaf_score - branch_score).abs() < f64::EPSILON,
        "leaf {} vs branch {}",
        leaf_score,
        branch_score
    );
}
