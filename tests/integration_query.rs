// Integration tests for `bo query`.
//
// All CI tests use mock LLM providers (no API keys needed).
// One manual-dogfooding smoke test (`live_api_query`) is kept `#[ignore]` for
// live-openai connectivity checks outside CI.

mod common;

use async_trait::async_trait;
use bo::cli::query;
use bo::domain::{Timestamp, Title, Url};
use bo::engine::llm::{LlmError, LlmProvider, LlmResponse, Message, Model, ProviderSchema};
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

fn test_model() -> Model {
    Model::parse("gpt-4o", bo::engine::llm::Provider::OpenAI).unwrap()
}

// ── mock provider ────────────────────────────────────────────────────────────

struct MockProvider {
    response: String,
}

impl MockProvider {
    fn new(answer: &str, cited_slugs: &[&str]) -> Self {
        let response = serde_json::json!({
            "answer": answer,
            "cited_slugs": cited_slugs,
        });
        MockProvider {
            response: response.to_string(),
        }
    }
}

#[async_trait]
impl LlmProvider for MockProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&ProviderSchema>,
        _reasoning_disabled: bool,
    ) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            content: self.response.clone(),
            finish_reason: bo::engine::llm::FinishReason::Stop,
        })
    }
}

// ── test fixtures ────────────────────────────────────────────────────────────

fn make_leaf(dir: &std::path::Path, filename: &str, title: &str, url: &str, body: &str) {
    let leaf_dir = dir.join("leaf");
    fs::create_dir_all(&leaf_dir).unwrap();

    let content = format!(
        "---\ntitle: \"{title}\"\nurl: \"{url}\"\ncollected_at: 2025-01-01T00:00:00Z\n---\n\n{body}"
    );

    fs::write(leaf_dir.join(filename), content).unwrap();
}

fn make_state(dir: &std::path::Path, entries: &[(&str, &str, &str, Option<&str>)]) {
    let leaves: Vec<_> = entries
        .iter()
        .map(|(file, title, url, summary)| {
            let slug_str = std::path::Path::new(file)
                .file_stem()
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let slug = bo::domain::Slug::parse(&slug_str)
                .unwrap_or_else(|_| bo::domain::Slug::generate(&slug_str, ""));
            bo::domain::Leaf {
                slug,
                file: file.to_string(),
                title: Title::parse(title).ok(),
                url: Url::parse(url).unwrap(),
                collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                summary: summary.map(str::to_string),
            }
        })
        .collect();
    common::write_state(
        dir,
        &bo::domain::state::TreeState {
            tree: bo::domain::state::TreeMetadata {
                name: "query-test".to_string(),
                created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                last_synthesized_at: None,
            },
            leaves,
            branches: Vec::new(),
        },
    );
}

fn setup_test_tree() -> TempDir {
    let dir = TempDir::new().unwrap();
    let tree = dir.path();

    make_leaf(
        tree,
        "rust-ownership.md",
        "Understanding Ownership",
        "https://doc.rust-lang.org/ownership",
        "Ownership is Rust's most unique feature. Each value has a variable that's its owner. There can only be one owner at a time. When the owner goes out of scope, the value is dropped.",
    );
    make_leaf(
        tree,
        "rust-borrowing.md",
        "References and Borrowing",
        "https://doc.rust-lang.org/borrowing",
        "References allow you to refer to some value without taking ownership. The rules: you can have either one mutable reference or any number of immutable references. References must always be valid.",
    );
    make_leaf(
        tree,
        "rust-lifetimes.md",
        "Lifetimes in Rust",
        "https://doc.rust-lang.org/lifetimes",
        "Every reference in Rust has a lifetime. Lifetimes are a way of describing the relationship between references. The borrow checker uses lifetimes to ensure references are valid.",
    );
    make_leaf(
        tree,
        "python-gc.md",
        "Python Garbage Collection",
        "https://docs.python.org/gc",
        "Python manages memory automatically using reference counting. When an object's reference count drops to zero, it is deallocated. A cycle detector handles circular references.",
    );
    make_leaf(
        tree,
        "go-concurrency.md",
        "Go Concurrency",
        "https://go.dev/concurrency",
        "Goroutines are lightweight threads managed by the Go runtime. Channels provide typed communication between goroutines. The select statement multiplexes channel operations.",
    );
    // Leaf without summary field
    make_leaf(
        tree,
        "rust-traits.md",
        "Rust Traits",
        "https://doc.rust-lang.org/traits",
        "Traits define shared behavior. A trait tells the Rust compiler about functionality a type must provide. Trait bounds constrain generic types to those implementing specific traits.",
    );

    make_state(
        tree,
        &[
            (
                "leaf/rust-ownership.md",
                "Understanding Ownership",
                "https://doc.rust-lang.org/ownership",
                Some("Rust ownership model ensures memory safety through compile-time checks"),
            ),
            (
                "leaf/rust-borrowing.md",
                "References and Borrowing",
                "https://doc.rust-lang.org/borrowing",
                Some("Borrowing allows references without taking ownership"),
            ),
            (
                "leaf/rust-lifetimes.md",
                "Lifetimes in Rust",
                "https://doc.rust-lang.org/lifetimes",
                Some("Lifetimes ensure references remain valid for their intended scope"),
            ),
            (
                "leaf/python-gc.md",
                "Python Garbage Collection",
                "https://docs.python.org/gc",
                Some("Python uses reference counting and cycle detection for memory management"),
            ),
            (
                "leaf/go-concurrency.md",
                "Go Concurrency",
                "https://go.dev/concurrency",
                Some("Go uses goroutines and channels for concurrent programming"),
            ),
            (
                "leaf/rust-traits.md",
                "Rust Traits",
                "https://doc.rust-lang.org/traits",
                None,
            ),
        ],
    );

    dir
}

// ── integration tests ────────────────────────────────────────────────────────

#[test]
fn full_pipeline_with_mock_provider() {
    let dir = setup_test_tree();

    // The question "how does Rust handle memory safety?" extracts terms: ["rust", "handle", "memory", "safety"]
    // rust-ownership matches (contains "rust", "memory", "safety")
    // rust-lifetimes matches (contains "rust")
    // Mock cites one valid and one invalid slug
    let provider = MockProvider::new(
        "Rust's ownership model ensures memory safety at compile time [[rust-ownership]]. Invalid citation here [[nonexistent]].",
        &["rust-ownership", "nonexistent"],
    );

    let result = query::run_with_provider_and_policy(
        dir.path(),
        "how does Rust handle memory safety?",
        &provider,
        &test_model(),
        query::QUERY_LLM_POLICY,
    )
    .unwrap();

    // Answer contains valid citation
    assert!(result.answer.contains("[[rust-ownership]]"));

    // Invalid citation stripped from prose
    assert!(!result.answer.contains("[[nonexistent]]"));
    // But the text "nonexistent" is preserved (brackets removed)
    assert!(result.answer.contains("nonexistent"));

    // Citations list only contains valid entries
    assert_eq!(result.citations.len(), 1);
    assert_eq!(result.citations[0].slug.as_str(), "rust-ownership");
    assert_eq!(
        result.citations[0].title.as_str(),
        "Understanding Ownership"
    );

    // Model recorded
    assert_eq!(result.model, "gpt-4o");

    // Leaves consulted is depth tier count (≤5)
    assert!(result.leaves_consulted <= 5);
    assert!(result.leaves_consulted > 0);
}

#[test]
fn json_output_is_schema_conformant() {
    let dir = setup_test_tree();

    let provider = MockProvider::new("Ownership is key [[rust-ownership]].", &["rust-ownership"]);

    let result = query::run_with_provider_and_policy(
        dir.path(),
        "what is ownership in Rust?",
        &provider,
        &test_model(),
        query::QUERY_LLM_POLICY,
    )
    .unwrap();

    let json_str = serde_json::to_string_pretty(&result).unwrap();
    let parsed: Value = serde_json::from_str(&json_str).unwrap();

    // Required fields present
    assert!(parsed["answer"].is_string());
    assert!(parsed["citations"].is_array());
    assert!(parsed["model"].is_string());
    assert!(parsed["leaves_consulted"].is_number());

    // Citation schema
    let citation = &parsed["citations"][0];
    assert!(citation["slug"].is_string());
    assert!(citation["title"].is_string());
    assert!(citation["file"].is_string());

    // No extra fields (additionalProperties: false)
    let obj = parsed.as_object().unwrap();
    assert_eq!(obj.len(), 4); // answer, citations, model, leaves_consulted
}

#[test]
fn no_relevant_sources_returns_error() {
    let dir = setup_test_tree();

    let provider = MockProvider::new("unused", &[]);

    let err = query::run_with_provider_and_policy(
        dir.path(),
        "quantum computing entanglement",
        &provider,
        &test_model(),
        query::QUERY_LLM_POLICY,
    )
    .unwrap_err();

    assert!(matches!(err, query::QueryError::NoResults));
    assert_eq!(err.exit_code(), 1);
}

#[test]
fn all_stop_words_returns_no_terms_error() {
    let dir = setup_test_tree();

    let provider = MockProvider::new("unused", &[]);

    let err = query::run_with_provider_and_policy(
        dir.path(),
        "what is it?",
        &provider,
        &test_model(),
        query::QUERY_LLM_POLICY,
    )
    .unwrap_err();

    assert!(matches!(err, query::QueryError::NoTerms));
    assert_eq!(err.exit_code(), 2);
}

#[test]
fn single_leaf_tree_works() {
    let dir = TempDir::new().unwrap();
    let tree = dir.path();

    make_leaf(
        tree,
        "only-leaf.md",
        "The Only Leaf",
        "https://example.com/only",
        "Rust is a systems programming language focused on safety and performance.",
    );
    make_state(
        tree,
        &[(
            "leaf/only-leaf.md",
            "The Only Leaf",
            "https://example.com/only",
            Some("This is the only document in the tree about Rust"),
        )],
    );

    let provider = MockProvider::new("Rust focuses on safety [[only-leaf]].", &["only-leaf"]);

    let result = query::run_with_provider_and_policy(
        tree,
        "what is Rust?",
        &provider,
        &test_model(),
        query::QUERY_LLM_POLICY,
    )
    .unwrap();

    assert_eq!(result.citations.len(), 1);
    assert_eq!(result.citations[0].slug.as_str(), "only-leaf");
    assert_eq!(result.leaves_consulted, 1);
}

#[test]
fn leaf_without_summary_still_retrieved() {
    let dir = setup_test_tree();

    // "traits" should match the leaf without a summary field
    let provider = MockProvider::new(
        "Traits define shared behavior [[rust-traits]].",
        &["rust-traits"],
    );

    let result = query::run_with_provider_and_policy(
        dir.path(),
        "explain Rust traits",
        &provider,
        &test_model(),
        query::QUERY_LLM_POLICY,
    )
    .unwrap();

    assert_eq!(result.citations.len(), 1);
    assert_eq!(result.citations[0].slug.as_str(), "rust-traits");
}

#[test]
fn zero_citations_returns_insufficient_sources() {
    let dir = setup_test_tree();

    // Provider returns an answer but cites nothing — retrieval matched but synthesis couldn't ground
    let provider = MockProvider::new("The PMNS matrix describes neutrino mixing parameters.", &[]);

    // "rust ownership" will match leaves, but provider returns zero citations
    let err = query::run_with_provider_and_policy(
        dir.path(),
        "what is Rust ownership?",
        &provider,
        &test_model(),
        query::QUERY_LLM_POLICY,
    )
    .unwrap_err();

    assert!(matches!(err, query::QueryError::InsufficientSources { .. }));
    assert_eq!(err.exit_code(), 1);
    assert!(err
        .to_string()
        .contains("could not produce a grounded answer"));
}

// ── manual dogfooding smoke test (live OpenAI; kept #[ignore]) ──

/// Manual smoke test of the real OpenAI provider. The query pipeline
/// (retrieval, synthesis, citation validation, JSON envelope) is covered
/// deterministically by the mock tests above. This only adds live-provider
/// connectivity and stays out of CI (no key, nondeterministic model output).
///
/// Run:
/// ```bash
/// OPENAI_API_KEY=sk-... cargo test --test integration_query -- --ignored live_api_query
/// ```
#[test]
#[ignore = "manual dogfooding: hits live OpenAI; pipeline covered by mock tests"]
fn live_api_query() {
    let dir = setup_test_tree();
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");

    let provider = bo::engine::llm::OpenAiProvider::new(&api_key);
    let result = query::run_with_provider_and_policy(
        dir.path(),
        "how does Rust ensure memory safety without a garbage collector?",
        &provider,
        &test_model(),
        query::QUERY_LLM_POLICY,
    )
    .unwrap();

    // Should produce an answer
    assert!(!result.answer.is_empty());
    // Should cite at least one source
    assert!(!result.citations.is_empty());
    // All citations should be valid leaf slugs
    for c in &result.citations {
        assert!(
            c.slug.starts_with("rust-")
                || c.slug.as_str() == "python-gc"
                || c.slug.as_str() == "go-concurrency",
            "unexpected citation: {}",
            c.slug
        );
    }
    println!("Answer:\n{}", query::render_human(&result));
}
