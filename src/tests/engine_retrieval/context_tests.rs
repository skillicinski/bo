use super::*;
use crate::engine::llm::{model::Model, Provider};
use crate::engine::retrieval::{DocKind, RetrievalDiagnostics, RetrievalError, RetrievedDoc};

fn test_model() -> Model {
    Model::parse("gpt-4o", Provider::OpenAI).unwrap()
}

#[test]
fn assemble_respects_depth_limit() {
    let leaves: Vec<RetrievedDoc> = (0..10)
        .map(|i| RetrievedDoc {
            kind: DocKind::Leaf,
            slug: format!("leaf-{}", i),
            title: format!("Leaf {}", i),
            url: format!("https://example.com/{}", i),
            file: format!("leaves/leaf-{}.md", i),
            summary: "Short summary.".to_string(),
            body: "Some body content here.".to_string(),
            score: 10.0 - i as f64,
            diagnostics: RetrievalDiagnostics::default(),
        })
        .collect();

    let (context, consulted) = assemble_context(&leaves, 10_000);

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
    let test_budget_words = 1000;
    let big_body = "word ".repeat(test_budget_words + 1000);
    let leaves = vec![RetrievedDoc {
        kind: DocKind::Leaf,
        slug: "big".to_string(),
        title: "Big Leaf".to_string(),
        url: "https://example.com/big".to_string(),
        file: "leaves/big.md".to_string(),
        summary: "Summary.".to_string(),
        body: big_body,
        score: 10.0,
        diagnostics: RetrievalDiagnostics::default(),
    }];

    let (context, consulted) = assemble_context(&leaves, test_budget_words);

    // Should not exceed budget significantly
    let word_count = context.split_whitespace().count();
    assert!(word_count <= test_budget_words + 100); // small overhead from formatting
    assert_eq!(consulted, 1);
}

// ── model budget tests ───────────────────────────────────────────────

#[test]
fn budget_known_128k_model() {
    let source_words = compute_context_budget(&test_model()).unwrap();
    let reserved_tokens = PROMPT_OVERHEAD_TOKENS + MAX_COMPLETION_TOKENS as usize;

    assert_eq!(
        source_words,
        ((128_000 - reserved_tokens) * TOKENS_TO_WORDS_NUMERATOR) / TOKENS_TO_WORDS_DENOMINATOR
    );
}

#[test]
fn budget_known_1m_model() {
    let source_words =
        compute_context_budget(&Model::parse("gpt-4.1-mini", Provider::OpenAI).unwrap()).unwrap();
    let reserved_tokens = PROMPT_OVERHEAD_TOKENS + MAX_COMPLETION_TOKENS as usize;

    assert_eq!(
        source_words,
        ((1_000_000 - reserved_tokens) * TOKENS_TO_WORDS_NUMERATOR) / TOKENS_TO_WORDS_DENOMINATOR
    );
}

#[test]
fn exhausted_budget_returns_error() {
    let reserved = PROMPT_OVERHEAD_TOKENS + MAX_COMPLETION_TOKENS as usize;
    let err = compute_context_budget_from_tokens("tiny", reserved).unwrap_err();

    assert!(matches!(err, RetrievalError::ContextBudgetExhausted { .. }));
}
