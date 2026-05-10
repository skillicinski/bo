use super::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

// ── term extraction tests ────────────────────────────────────────────

#[test]
fn extract_basic_question() {
    let terms = extract_terms("what are the tradeoffs of Rust's ownership model?").unwrap();
    assert_eq!(terms, vec!["tradeoffs", "rust", "ownership", "model"]);
}

#[test]
fn extract_single_word() {
    let terms = extract_terms("ownership").unwrap();
    assert_eq!(terms, vec!["ownership"]);
}

#[test]
fn extract_all_stop_words_returns_error() {
    let err = extract_terms("what is it?").unwrap_err();
    assert!(matches!(err, QueryError::NoTerms));
}

#[test]
fn extract_strips_possessives() {
    let terms = extract_terms("Rust's borrow checker").unwrap();
    assert_eq!(terms, vec!["rust", "borrow", "checker"]);
}

#[test]
fn extract_drops_short_terms() {
    // "a" and "I" are < 2 chars and should be dropped
    let terms = extract_terms("a big I see").unwrap();
    assert_eq!(terms, vec!["big", "see"]);
}

#[test]
fn extract_strips_boundary_punctuation() {
    let terms = extract_terms("(memory) safety! \"lifetimes\"").unwrap();
    assert_eq!(terms, vec!["memory", "safety", "lifetimes"]);
}

#[test]
fn extract_unicode_possessive() {
    // Smart quote possessive: Rust\u{2019}s
    let terms = extract_terms("Rust\u{2019}s ownership").unwrap();
    assert_eq!(terms, vec!["rust", "ownership"]);
}

// ── retrieval tests ──────────────────────────────────────────────────

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

    fs::write(leaves_dir.join(filename), content).unwrap();
}

fn make_index(dir: &Path, entries: &[(&str, &str, &str)]) {
    let mut lines = String::new();
    for (file, title, url) in entries {
        lines.push_str(&format!(
            "{{\"file\":\"{}\",\"title\":\"{}\",\"url\":\"{}\"}}\n",
            file, title, url
        ));
    }
    fs::write(dir.join("index.jsonl"), lines).unwrap();
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

    make_index(
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
    let results = retrieve_leaves(tree, &terms).unwrap();

    // ownership leaf should rank highest (both terms match densely)
    assert_eq!(results[0].slug, "ownership");
    // lifetimes should match (contains "rust")
    assert!(results.iter().any(|r| r.slug == "lifetimes"));
    // cooking should NOT match
    assert!(!results.iter().any(|r| r.slug == "cooking"));
}

#[test]
fn retrieve_empty_tree_returns_error() {
    let dir = TempDir::new().unwrap();
    let tree = dir.path();
    make_index(tree, &[]);

    let err = retrieve_leaves(tree, &["rust".to_string()]).unwrap_err();
    assert!(matches!(err, QueryError::EmptyTree));
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
    make_index(
        tree,
        &[(
            "leaves/cooking.md",
            "Cooking Tips",
            "https://example.com/cooking",
        )],
    );

    let err = retrieve_leaves(tree, &["rust".to_string()]).unwrap_err();
    assert!(matches!(err, QueryError::NoResults));
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
    make_index(
        tree,
        &[(
            "leaves/nosummary.md",
            "No Summary Leaf",
            "https://example.com/ns",
        )],
    );

    let terms = vec!["rust".to_string()];
    let results = retrieve_leaves(tree, &terms).unwrap();

    assert_eq!(results[0].slug, "nosummary");
    // Summary should be the body fallback (body is short, so full body used)
    assert!(results[0].summary.contains("Rust programming"));
}

// ── helper tests ─────────────────────────────────────────────────────

#[test]
fn slug_from_file_strips_dir_and_extension() {
    assert_eq!(slug_from_file("leaves/foo-bar.md"), "foo-bar");
    assert_eq!(slug_from_file("leaves/sub/deep.md"), "deep");
    assert_eq!(slug_from_file("simple.md"), "simple");
}

// ── citation validation tests ────────────────────────────────────────

#[test]
fn validate_strips_invalid_citations() {
    let retrieved = vec![RetrievedLeaf {
        slug: "valid-leaf".to_string(),
        title: "Valid Leaf".to_string(),
        url: "https://example.com".to_string(),
        file: "leaves/valid-leaf.md".to_string(),
        summary: "summary".to_string(),
        body: "body".to_string(),
        score: 1.0,
    }];

    let response = SynthesisResponse {
        answer: "Answer cites [[valid-leaf]] and [[hallucinated]] sources.".to_string(),
        cited_slugs: vec!["valid-leaf".to_string(), "hallucinated".to_string()],
    };

    let (answer, citations) = validate_citations(response, &retrieved);

    // Invalid slug removed from prose
    assert!(answer.contains("[[valid-leaf]]"));
    assert!(!answer.contains("[[hallucinated]]"));
    assert!(answer.contains("hallucinated")); // text preserved, brackets removed

    // Invalid slug removed from citations list
    assert_eq!(citations.len(), 1);
    assert_eq!(citations[0].slug, "valid-leaf");
}

#[test]
fn validate_preserves_all_valid_citations() {
    let retrieved = vec![
        RetrievedLeaf {
            slug: "leaf-a".to_string(),
            title: "Leaf A".to_string(),
            url: "https://a.com".to_string(),
            file: "leaves/leaf-a.md".to_string(),
            summary: "s".to_string(),
            body: "b".to_string(),
            score: 1.0,
        },
        RetrievedLeaf {
            slug: "leaf-b".to_string(),
            title: "Leaf B".to_string(),
            url: "https://b.com".to_string(),
            file: "leaves/leaf-b.md".to_string(),
            summary: "s".to_string(),
            body: "b".to_string(),
            score: 0.5,
        },
    ];

    let response = SynthesisResponse {
        answer: "See [[leaf-a]] and [[leaf-b]] for details.".to_string(),
        cited_slugs: vec!["leaf-a".to_string(), "leaf-b".to_string()],
    };

    let (answer, citations) = validate_citations(response, &retrieved);

    assert!(answer.contains("[[leaf-a]]"));
    assert!(answer.contains("[[leaf-b]]"));
    assert_eq!(citations.len(), 2);
}

// ── context assembly tests ───────────────────────────────────────────

#[test]
fn assemble_respects_depth_limit() {
    let leaves: Vec<RetrievedLeaf> = (0..10)
        .map(|i| RetrievedLeaf {
            slug: format!("leaf-{}", i),
            title: format!("Leaf {}", i),
            url: format!("https://example.com/{}", i),
            file: format!("leaves/leaf-{}.md", i),
            summary: "Short summary.".to_string(),
            body: "Some body content here.".to_string(),
            score: 10.0 - i as f64,
        })
        .collect();

    let (context, consulted) = assemble_context(&leaves);

    // All 10 appear in breadth tier
    for i in 0..10 {
        assert!(context.contains(&format!("[[leaf-{}]]", i)));
    }
    // Only top 5 get full body
    assert_eq!(consulted, 5);
    assert!(context.contains("### [[leaf-0]]"));
    assert!(context.contains("### [[leaf-4]]"));
    assert!(!context.contains("### [[leaf-5]]"));
}

#[test]
fn assemble_truncates_on_word_budget() {
    // Create a leaf with a massive body
    let big_body = "word ".repeat(TOKEN_BUDGET_WORDS + 1000);
    let leaves = vec![RetrievedLeaf {
        slug: "big".to_string(),
        title: "Big Leaf".to_string(),
        url: "https://example.com/big".to_string(),
        file: "leaves/big.md".to_string(),
        summary: "Summary.".to_string(),
        body: big_body,
        score: 10.0,
    }];

    let (context, consulted) = assemble_context(&leaves);

    // Should not exceed budget significantly
    let word_count = context.split_whitespace().count();
    assert!(word_count <= TOKEN_BUDGET_WORDS + 100); // small overhead from formatting
    assert_eq!(consulted, 1);
}
