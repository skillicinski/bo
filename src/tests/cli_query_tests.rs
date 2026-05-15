use super::*;
use async_trait::async_trait;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

use crate::engine::llm::{FinishReason, LlmProvider, LlmResponse};

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
    let bo_dir = dir.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    fs::write(bo_dir.join("index.jsonl"), lines).unwrap();
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

fn retrieved_leaf(slug: &str) -> RetrievedLeaf {
    RetrievedLeaf {
        slug: slug.to_string(),
        title: format!("Title for {}", slug),
        url: format!("https://example.com/{}", slug),
        file: format!("leaves/{}.md", slug),
        summary: "summary".to_string(),
        body: "body".to_string(),
        score: 1.0,
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
    assert_eq!(citations[0].slug, "valid-leaf");
}

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
    assert_eq!(citations[0].slug, "leaf-a");
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
    assert_eq!(citations[0].slug, "leaf-a");
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

// ── model budget tests ───────────────────────────────────────────────

#[test]
fn query_budget_known_128k_model() {
    let budget = compute_query_context_budget("gpt-4o").unwrap();

    assert_eq!(budget.model, "gpt-4o");
    assert_eq!(budget.context_tokens, 128_000);
    assert_eq!(
        budget.reserved_tokens,
        QUERY_PROMPT_OVERHEAD_TOKENS + QUERY_MAX_COMPLETION_TOKENS as usize
    );
    assert_eq!(
        budget.source_words,
        ((128_000 - budget.reserved_tokens) * TOKENS_TO_WORDS_NUMERATOR)
            / TOKENS_TO_WORDS_DENOMINATOR
    );
}

#[test]
fn query_budget_known_1m_model() {
    let budget = compute_query_context_budget("gpt-4.1-mini").unwrap();

    assert_eq!(budget.model, "gpt-4.1-mini");
    assert_eq!(budget.context_tokens, 1_000_000);
    assert_eq!(
        budget.source_words,
        ((1_000_000 - budget.reserved_tokens) * TOKENS_TO_WORDS_NUMERATOR)
            / TOKENS_TO_WORDS_DENOMINATOR
    );
}

struct CountingProvider {
    calls: AtomicUsize,
}

impl CountingProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for CountingProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            content: r#"{"answer":"unused","cited_slugs":[]}"#.to_string(),
            finish_reason: FinishReason::Stop,
        })
    }
}

#[test]
fn unknown_model_fails_before_provider_invocation() {
    let dir = TempDir::new().unwrap();
    let provider = CountingProvider::new();

    let err =
        run_with_provider(dir.path(), "what is rust", &provider, "unknown-model").unwrap_err();

    assert!(matches!(err, QueryError::UnknownModelContext { .. }));
    assert_eq!(provider.calls(), 0);
}

#[test]
fn exhausted_budget_returns_error() {
    let reserved = QUERY_PROMPT_OVERHEAD_TOKENS + QUERY_MAX_COMPLETION_TOKENS as usize;
    let err = compute_query_context_budget_from_tokens("tiny", reserved).unwrap_err();

    assert!(matches!(err, QueryError::ContextBudgetExhausted { .. }));
}

struct FlakyQueryProvider {
    calls: AtomicUsize,
    fail_attempts: usize,
    finish_reason: FinishReason,
}

impl FlakyQueryProvider {
    fn new(fail_attempts: usize, finish_reason: FinishReason) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fail_attempts,
            finish_reason,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for FlakyQueryProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call <= self.fail_attempts {
            return Err(LlmError::Network("temporary failure".to_string()));
        }
        Ok(LlmResponse {
            content: r#"{"answer":"Rust is safe [[only-leaf]].","cited_slugs":["only-leaf"]}"#
                .to_string(),
            finish_reason: self.finish_reason.clone(),
        })
    }
}

struct HangingQueryProvider {
    calls: AtomicUsize,
}

impl HangingQueryProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for HangingQueryProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(5)).await;
        Ok(LlmResponse {
            content: "{}".to_string(),
            finish_reason: FinishReason::Stop,
        })
    }
}

fn single_leaf_query_tree() -> TempDir {
    let dir = TempDir::new().unwrap();
    make_leaf(
        dir.path(),
        "only-leaf.md",
        "Only Leaf",
        "https://example.com/only",
        Some("Rust safety"),
        "Rust is a language focused on safety.",
    );
    make_index(
        dir.path(),
        &[(
            "leaves/only-leaf.md",
            "Only Leaf",
            "https://example.com/only",
        )],
    );
    dir
}

fn short_query_policy(max_attempts: usize) -> LlmCallPolicy {
    LlmCallPolicy {
        timeout: Duration::from_millis(20),
        max_attempts,
        initial_backoff: Duration::ZERO,
    }
}

#[test]
fn query_retries_transient_failure_and_succeeds() {
    let dir = single_leaf_query_tree();
    let provider = FlakyQueryProvider::new(1, FinishReason::Stop);

    let result = run_with_provider_and_policy(
        dir.path(),
        "what is rust safety",
        &provider,
        "gpt-4o",
        short_query_policy(3),
    )
    .unwrap();

    assert_eq!(provider.calls(), 2);
    assert_eq!(result.citations[0].slug, "only-leaf");
}

#[test]
fn query_timeout_returns_llm_error() {
    let dir = single_leaf_query_tree();
    let provider = HangingQueryProvider::new();

    let err = run_with_provider_and_policy(
        dir.path(),
        "what is rust safety",
        &provider,
        "gpt-4o",
        short_query_policy(1),
    )
    .unwrap_err();

    assert_eq!(provider.calls(), 1);
    assert!(matches!(
        err,
        QueryError::Llm(LlmError::RetryExhausted { .. })
    ));
}

#[test]
fn query_length_finish_reason_fails_before_parse() {
    let dir = single_leaf_query_tree();
    let provider = FlakyQueryProvider::new(0, FinishReason::Length);

    let err = run_with_provider_and_policy(
        dir.path(),
        "what is rust safety",
        &provider,
        "gpt-4o",
        short_query_policy(1),
    )
    .unwrap_err();

    assert!(matches!(err, QueryError::Truncated));
}

#[test]
fn query_content_filter_finish_reason_fails_before_parse() {
    let dir = single_leaf_query_tree();
    let provider = FlakyQueryProvider::new(0, FinishReason::ContentFilter);

    let err = run_with_provider_and_policy(
        dir.path(),
        "what is rust safety",
        &provider,
        "gpt-4o",
        short_query_policy(1),
    )
    .unwrap_err();

    assert!(matches!(err, QueryError::ContentFilter));
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
    let leaves = vec![RetrievedLeaf {
        slug: "big".to_string(),
        title: "Big Leaf".to_string(),
        url: "https://example.com/big".to_string(),
        file: "leaves/big.md".to_string(),
        summary: "Summary.".to_string(),
        body: big_body,
        score: 10.0,
    }];

    let (context, consulted) = assemble_context(&leaves, test_budget_words);

    // Should not exceed budget significantly
    let word_count = context.split_whitespace().count();
    assert!(word_count <= test_budget_words + 100); // small overhead from formatting
    assert_eq!(consulted, 1);
}

// ── insufficient sources (zero-citation) tests ───────────────────────────────

struct ZeroCitationProvider;

#[async_trait]
impl LlmProvider for ZeroCitationProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            content: r#"{"answer":"The sources do not cover this topic.","cited_slugs":[]}"#
                .to_string(),
            finish_reason: FinishReason::Stop,
        })
    }
}

#[test]
fn zero_citations_returns_insufficient_sources_error() {
    let dir = single_leaf_query_tree();
    let provider = ZeroCitationProvider;

    let err = run_with_provider_and_policy(
        dir.path(),
        "what is rust safety",
        &provider,
        "gpt-4o",
        short_query_policy(1),
    )
    .unwrap_err();

    match &err {
        QueryError::InsufficientSources { leaves_consulted } => {
            assert_eq!(*leaves_consulted, 1);
        }
        other => panic!("expected InsufficientSources, got: {:?}", other),
    }
    assert_eq!(err.exit_code(), 1);
    assert!(
        err.to_string().contains("searched 1 sources"),
        "display: {}",
        err
    );
}

#[test]
fn one_valid_citation_returns_ok() {
    let dir = single_leaf_query_tree();
    let provider = FlakyQueryProvider::new(0, FinishReason::Stop);

    let result = run_with_provider_and_policy(
        dir.path(),
        "what is rust safety",
        &provider,
        "gpt-4o",
        short_query_policy(1),
    )
    .unwrap();

    assert_eq!(result.citations.len(), 1);
    assert_eq!(result.citations[0].slug, "only-leaf");
}
