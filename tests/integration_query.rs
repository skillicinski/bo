// Integration tests for `bo query`.
//
// Uses a mock LlmProvider to test the full pipeline without network calls.
// Tests that require a live API key are marked `#[ignore]`.

use async_trait::async_trait;
use bo::cli::query;
use bo::engine::llm::{LlmError, LlmProvider, LlmResponse, Message};
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

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
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        Ok(LlmResponse {
            content: self.response.clone(),
            finish_reason: bo::engine::llm::FinishReason::Stop,
        })
    }
}

// ── test fixtures ────────────────────────────────────────────────────────────

fn make_leaf(
    dir: &std::path::Path,
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

fn make_index(dir: &std::path::Path, entries: &[(&str, &str, &str)]) {
    let mut lines = String::new();
    for (file, title, url) in entries {
        lines.push_str(&format!(
            "{{\"file\":\"{}\",\"title\":\"{}\",\"url\":\"{}\"}}\n",
            file, title, url
        ));
    }
    fs::write(dir.join("index.jsonl"), lines).unwrap();
}

fn setup_test_tree() -> TempDir {
    let dir = TempDir::new().unwrap();
    let tree = dir.path();

    make_leaf(
        tree,
        "rust-ownership.md",
        "Understanding Ownership",
        "https://doc.rust-lang.org/ownership",
        Some("Rust ownership model ensures memory safety through compile-time checks"),
        "Ownership is Rust's most unique feature. Each value has a variable that's its owner. There can only be one owner at a time. When the owner goes out of scope, the value is dropped.",
    );
    make_leaf(
        tree,
        "rust-borrowing.md",
        "References and Borrowing",
        "https://doc.rust-lang.org/borrowing",
        Some("Borrowing allows references without taking ownership"),
        "References allow you to refer to some value without taking ownership. The rules: you can have either one mutable reference or any number of immutable references. References must always be valid.",
    );
    make_leaf(
        tree,
        "rust-lifetimes.md",
        "Lifetimes in Rust",
        "https://doc.rust-lang.org/lifetimes",
        Some("Lifetimes ensure references remain valid for their intended scope"),
        "Every reference in Rust has a lifetime. Lifetimes are a way of describing the relationship between references. The borrow checker uses lifetimes to ensure references are valid.",
    );
    make_leaf(
        tree,
        "python-gc.md",
        "Python Garbage Collection",
        "https://docs.python.org/gc",
        Some("Python uses reference counting and cycle detection for memory management"),
        "Python manages memory automatically using reference counting. When an object's reference count drops to zero, it is deallocated. A cycle detector handles circular references.",
    );
    make_leaf(
        tree,
        "go-concurrency.md",
        "Go Concurrency",
        "https://go.dev/concurrency",
        Some("Go uses goroutines and channels for concurrent programming"),
        "Goroutines are lightweight threads managed by the Go runtime. Channels provide typed communication between goroutines. The select statement multiplexes channel operations.",
    );
    // Leaf without summary field
    make_leaf(
        tree,
        "rust-traits.md",
        "Rust Traits",
        "https://doc.rust-lang.org/traits",
        None,
        "Traits define shared behavior. A trait tells the Rust compiler about functionality a type must provide. Trait bounds constrain generic types to those implementing specific traits.",
    );

    make_index(
        tree,
        &[
            (
                "leaves/rust-ownership.md",
                "Understanding Ownership",
                "https://doc.rust-lang.org/ownership",
            ),
            (
                "leaves/rust-borrowing.md",
                "References and Borrowing",
                "https://doc.rust-lang.org/borrowing",
            ),
            (
                "leaves/rust-lifetimes.md",
                "Lifetimes in Rust",
                "https://doc.rust-lang.org/lifetimes",
            ),
            (
                "leaves/python-gc.md",
                "Python Garbage Collection",
                "https://docs.python.org/gc",
            ),
            (
                "leaves/go-concurrency.md",
                "Go Concurrency",
                "https://go.dev/concurrency",
            ),
            (
                "leaves/rust-traits.md",
                "Rust Traits",
                "https://doc.rust-lang.org/traits",
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

    let result = query::run_with_provider(
        dir.path(),
        "how does Rust handle memory safety?",
        &provider,
        "gpt-4o",
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
    assert_eq!(result.citations[0].slug, "rust-ownership");
    assert_eq!(result.citations[0].title, "Understanding Ownership");

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

    let result = query::run_with_provider(
        dir.path(),
        "what is ownership in Rust?",
        &provider,
        "gpt-4o",
    )
    .unwrap();

    let json_str = query::render_json(&result).unwrap();
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

    let err = query::run_with_provider(
        dir.path(),
        "quantum computing entanglement",
        &provider,
        "gpt-4o",
    )
    .unwrap_err();

    assert!(matches!(err, query::QueryError::NoResults));
    assert_eq!(err.exit_code(), 1);
}

#[test]
fn all_stop_words_returns_no_terms_error() {
    let dir = setup_test_tree();

    let provider = MockProvider::new("unused", &[]);

    let err = query::run_with_provider(dir.path(), "what is it?", &provider, "gpt-4o").unwrap_err();

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
        Some("This is the only document in the tree about Rust"),
        "Rust is a systems programming language focused on safety and performance.",
    );
    make_index(
        tree,
        &[(
            "leaves/only-leaf.md",
            "The Only Leaf",
            "https://example.com/only",
        )],
    );

    let provider = MockProvider::new("Rust focuses on safety [[only-leaf]].", &["only-leaf"]);

    let result = query::run_with_provider(tree, "what is Rust?", &provider, "gpt-4o").unwrap();

    assert_eq!(result.citations.len(), 1);
    assert_eq!(result.citations[0].slug, "only-leaf");
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

    let result =
        query::run_with_provider(dir.path(), "explain Rust traits", &provider, "gpt-4o").unwrap();

    assert_eq!(result.citations.len(), 1);
    assert_eq!(result.citations[0].slug, "rust-traits");
}

// ── live API test (ignored by default) ───────────────────────────────────────

#[test]
#[ignore]
fn live_api_query() {
    let dir = setup_test_tree();
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY must be set");

    let result = query::run(
        dir.path(),
        "how does Rust ensure memory safety without a garbage collector?",
        &api_key,
        "gpt-4o",
    )
    .unwrap();

    // Should produce an answer
    assert!(!result.answer.is_empty());
    // Should cite at least one source
    assert!(!result.citations.is_empty());
    // All citations should be valid leaf slugs
    for c in &result.citations {
        assert!(
            c.slug.starts_with("rust-") || c.slug == "python-gc" || c.slug == "go-concurrency",
            "unexpected citation: {}",
            c.slug
        );
    }
    println!("Answer:\n{}", query::render_human(&result));
}
