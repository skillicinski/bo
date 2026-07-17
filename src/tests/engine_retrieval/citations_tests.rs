use crate::engine::retrieval::{
    validate_citations, DocKind, RetrievalDiagnostics, RetrievedDoc, SynthesisResponse,
};

fn retrieved_leaf(slug: &str) -> RetrievedDoc {
    RetrievedDoc {
        kind: DocKind::Leaf,
        slug: slug.to_string(),
        title: format!("Title for {}", slug),
        url: format!("https://example.com/{}", slug),
        file: format!("leaves/{}.md", slug),
        summary: "summary".to_string(),
        body: "body".to_string(),
        score: 1.0,
        diagnostics: RetrievalDiagnostics::default(),
    }
}

#[test]
fn validate_preserves_valid_wikilinks_exactly() {
    let retrieved = vec![retrieved_leaf("valid-leaf")];
    let response = SynthesisResponse {
        answer: "Answer cites [[valid-leaf]] exactly.".to_string(),
        cited_slugs: vec!["valid-leaf".to_string()],
    };

    let (answer, citations) = validate_citations(response, &retrieved);

    assert_eq!(answer, "Answer cites [[valid-leaf]] exactly.");
    assert_eq!(citations.len(), 1);
    assert_eq!(citations[0].slug.as_str(), "valid-leaf");
}

#[test]
fn validate_strips_invalid_citations() {
    let retrieved = vec![RetrievedDoc {
        kind: DocKind::Leaf,
        slug: "valid-leaf".to_string(),
        title: "Valid Leaf".to_string(),
        url: "https://example.com".to_string(),
        file: "leaves/valid-leaf.md".to_string(),
        summary: "summary".to_string(),
        body: "body".to_string(),
        score: 1.0,
        diagnostics: RetrievalDiagnostics::default(),
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
    assert_eq!(citations[0].slug.as_str(), "valid-leaf");
}

#[test]
fn validate_preserves_adjacent_valid_wikilinks() {
    let retrieved = vec![retrieved_leaf("leaf-a"), retrieved_leaf("leaf-b")];
    let response = SynthesisResponse {
        answer: "Compare [[leaf-a]][[leaf-b]].".to_string(),
        cited_slugs: vec!["leaf-a".to_string(), "leaf-b".to_string()],
    };

    let (answer, citations) = validate_citations(response, &retrieved);

    assert_eq!(answer, "Compare [[leaf-a]][[leaf-b]].");
    assert_eq!(
        citations
            .iter()
            .map(|c| c.slug.as_str())
            .collect::<Vec<_>>(),
        vec!["leaf-a", "leaf-b"]
    );
}

#[test]
fn validate_leaves_malformed_nested_empty_and_unclosed_wikilinks_unchanged() {
    let retrieved = vec![retrieved_leaf("leaf-a")];
    let response = SynthesisResponse {
        answer: "Keep [[ and [[foo and [[]] and [[foo] and [[foo[[bar]] but keep [[leaf-a]]."
            .to_string(),
        cited_slugs: vec!["leaf-a".to_string()],
    };

    let (answer, citations) = validate_citations(response, &retrieved);

    assert_eq!(
        answer,
        "Keep [[ and [[foo and [[]] and [[foo] and [[foo[[bar]] but keep [[leaf-a]]."
    );
    assert_eq!(citations.len(), 1);
    assert_eq!(citations[0].slug.as_str(), "leaf-a");
}

#[test]
fn validate_includes_valid_prose_wikilink_missing_from_cited_slugs() {
    let retrieved = vec![retrieved_leaf("leaf-a")];
    let response = SynthesisResponse {
        answer: "The answer cites [[leaf-a]] in prose only.".to_string(),
        cited_slugs: Vec::new(),
    };

    let (_answer, citations) = validate_citations(response, &retrieved);

    assert_eq!(citations.len(), 1);
    assert_eq!(citations[0].slug.as_str(), "leaf-a");
}

#[test]
fn validate_dedupes_citations_in_prose_then_structured_order() {
    let retrieved = vec![
        retrieved_leaf("leaf-a"),
        retrieved_leaf("leaf-b"),
        retrieved_leaf("leaf-c"),
    ];
    let response = SynthesisResponse {
        answer: "First [[leaf-b]], then [[leaf-a]], then again [[leaf-b]].".to_string(),
        cited_slugs: vec![
            "leaf-c".to_string(),
            "leaf-a".to_string(),
            "leaf-c".to_string(),
        ],
    };

    let (_answer, citations) = validate_citations(response, &retrieved);

    assert_eq!(
        citations
            .iter()
            .map(|c| c.slug.as_str())
            .collect::<Vec<_>>(),
        vec!["leaf-b", "leaf-a", "leaf-c"]
    );
}

#[test]
fn validate_preserves_all_valid_citations() {
    let retrieved = vec![
        RetrievedDoc {
            kind: DocKind::Leaf,
            slug: "leaf-a".to_string(),
            title: "Leaf A".to_string(),
            url: "https://a.com".to_string(),
            file: "leaves/leaf-a.md".to_string(),
            summary: "s".to_string(),
            body: "b".to_string(),
            score: 1.0,
            diagnostics: RetrievalDiagnostics::default(),
        },
        RetrievedDoc {
            kind: DocKind::Leaf,
            slug: "leaf-b".to_string(),
            title: "Leaf B".to_string(),
            url: "https://b.com".to_string(),
            file: "leaves/leaf-b.md".to_string(),
            summary: "s".to_string(),
            body: "b".to_string(),
            score: 0.5,
            diagnostics: RetrievalDiagnostics::default(),
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
