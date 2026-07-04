use super::*;
use crate::domain::{Slug, Timestamp};
use crate::engine::llm::{
    model::Model, FinishReason, LlmProvider, LlmResponse, NormalizedSchema, Provider,
};
use crate::engine::retrieval::DocKind;
use async_trait::async_trait;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

fn test_model() -> Model {
    Model::parse("gpt-4o", Provider::OpenAI).unwrap()
}

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

// ── query error metadata tests ───────────────────────────────────────

#[test]
fn no_answer_errors_expose_codes_exit_codes_next_steps_and_details() {
    let cases = vec![
        (QueryError::EmptyTree, "empty_tree"),
        (QueryError::NoResults, "no_results"),
        (
            QueryError::LowRelevance {
                reason: LowRelevanceReason::WeakMatches,
                matched_sources: 2,
            },
            "low_relevance",
        ),
        (
            QueryError::InsufficientSources {
                leaves_consulted: 3,
            },
            "insufficient_sources",
        ),
    ];

    for (error, code) in cases {
        assert_eq!(error.code(), code);
        assert_eq!(error.exit_code(), 1);
        assert!(error.next_step().is_some(), "{error:?} missing next step");
        assert!(
            error.details().get("next_step").is_some(),
            "{error:?} missing JSON next_step"
        );
    }
}

#[test]
fn low_relevance_details_include_reason_and_matched_sources() {
    let error = QueryError::LowRelevance {
        reason: LowRelevanceReason::GenericQuery,
        matched_sources: 8,
    };

    let details = error.details();

    assert_eq!(error.code(), "low_relevance");
    assert_eq!(details["reason"], "generic_query");
    assert_eq!(details["matched_sources"], 8);
    assert!(details["next_step"].as_str().unwrap().contains("specific"));
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

fn make_manifest(dir: &Path, entries: &[(&str, &str, &str)]) {
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
            crate::domain::manifest::LeafRecord {
                slug: Slug::generate(&Path::new(file).file_stem().unwrap().to_string_lossy(), ""),
                file: file.to_string(),
                title: title.to_string(),
                url: (url).to_string(),
                collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                summary,
            }
        })
        .collect();
    let bo_dir = dir.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    crate::engine::manifest::write(
        &bo_dir.join("manifest.json"),
        &crate::domain::manifest::Manifest {
            tree: crate::domain::manifest::TreeMeta {
                name: "query".to_string(),
                created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves,
            branches: Vec::new(),
        },
    )
    .unwrap();
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

    make_manifest(
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
    make_manifest(tree, &[]);

    let err = retrieve_docs(tree, &["rust".to_string()]).unwrap_err();
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
    make_manifest(
        tree,
        &[(
            "leaves/cooking.md",
            "Cooking Tips",
            "https://example.com/cooking",
        )],
    );

    let err = retrieve_docs(tree, &["rust".to_string()]).unwrap_err();
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
    make_manifest(
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
// `bo compile`'s synthesized output is invisible at query time.
#[test]
fn retrieve_returns_compiled_branch_when_no_leaf_matches() {
    use crate::domain::manifest::{BranchRecord, LeafRecord, Manifest, TreeMeta};
    use crate::domain::{Title, Url};

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
    fs::create_dir_all(tree.join("branches")).unwrap();
    fs::write(
        tree.join("branches/rust-ownership.md"),
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

    let manifest = Manifest {
        tree: TreeMeta {
            name: "query".to_string(),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2025-01-01T00:00:00Z").unwrap()),
        },
        leaves: vec![
            LeafRecord {
                slug: Slug::generate("cooking", ""),
                file: "leaves/cooking.md".to_string(),
                title: Title::from("Cooking Tips"),
                url: Url::from("https://example.com/cooking"),
                collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                summary: Some("How to cook".to_string()),
            },
            LeafRecord {
                slug: Slug::generate("sports", ""),
                file: "leaves/sports.md".to_string(),
                title: Title::from("Sports News"),
                url: Url::from("https://example.com/sports"),
                collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                summary: Some("Match reports".to_string()),
            },
        ],
        branches: vec![BranchRecord {
            slug: Slug::generate("Rust Ownership", ""),
            file: "branches/rust-ownership.md".to_string(),
            title: Title::from("Rust Ownership"),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            updated_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            leaves: Vec::new(),
        }],
    };
    let bo_dir = tree.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    crate::engine::manifest::write(&bo_dir.join("manifest.json"), &manifest).unwrap();

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
        file: "branches/rust-ownership.md".to_string(),
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
fn diagnostics_use_token_matches_not_substrings() {
    let terms = vec!["rust".to_string()];
    let diagnostics = compute_retrieval_diagnostics(
        "Trust Building",
        "Trustworthy teams",
        "A trust exercise",
        &terms,
    );

    assert_eq!(diagnostics.total_hits, 0);
    assert_eq!(diagnostics.matched_terms, 0);
}

#[test]
fn diagnostics_capture_focused_title_and_summary_matches() {
    let terms = vec!["rust".to_string(), "safety".to_string()];
    let diagnostics = compute_retrieval_diagnostics(
        "Rust Safety",
        "Rust ownership safety",
        "Memory safety without a garbage collector",
        &terms,
    );

    assert_eq!(diagnostics.matched_terms, 2);
    assert_eq!(diagnostics.title_hits, 2);
    assert_eq!(diagnostics.summary_hits, 2);
    assert_eq!(diagnostics.matched_non_generic_terms, 2);
}

// ── helper tests ─────────────────────────────────────────────────────

// (slug_from_file removed: slugs now come from manifest LeafRecord.slug)

// ── citation validation tests ────────────────────────────────────────

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

// ── model budget tests ───────────────────────────────────────────────

#[test]
fn query_budget_known_128k_model() {
    let source_words = compute_query_context_budget(&test_model()).unwrap();
    let reserved_tokens = QUERY_PROMPT_OVERHEAD_TOKENS + QUERY_MAX_COMPLETION_TOKENS as usize;

    assert_eq!(
        source_words,
        ((128_000 - reserved_tokens) * TOKENS_TO_WORDS_NUMERATOR) / TOKENS_TO_WORDS_DENOMINATOR
    );
}

#[test]
fn query_budget_known_1m_model() {
    let source_words =
        compute_query_context_budget(&Model::parse("gpt-4.1-mini", Provider::OpenAI).unwrap())
            .unwrap();
    let reserved_tokens = QUERY_PROMPT_OVERHEAD_TOKENS + QUERY_MAX_COMPLETION_TOKENS as usize;

    assert_eq!(
        source_words,
        ((1_000_000 - reserved_tokens) * TOKENS_TO_WORDS_NUMERATOR) / TOKENS_TO_WORDS_DENOMINATOR
    );
}

#[test]
fn unknown_model_rejected_at_parse_boundary() {
    // Model validation now happens at parse time (config boundary),
    // not at query execution time. An invalid model string cannot reach
    // the query pipeline as a &Model.
    assert!(Model::parse("unknown-model", Provider::OpenAI).is_err());
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
        _response_schema: Option<&NormalizedSchema>,
        _reasoning_disabled: bool,
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
        _response_schema: Option<&NormalizedSchema>,
        _reasoning_disabled: bool,
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
    make_manifest(
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
        &test_model(),
        short_query_policy(3),
    )
    .unwrap();

    assert_eq!(provider.calls(), 2);
    assert_eq!(result.citations[0].slug.as_str(), "only-leaf");
}

#[test]
fn query_timeout_returns_llm_error() {
    let dir = single_leaf_query_tree();
    let provider = HangingQueryProvider::new();

    let err = run_with_provider_and_policy(
        dir.path(),
        "what is rust safety",
        &provider,
        &test_model(),
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
        &test_model(),
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
        &test_model(),
        short_query_policy(1),
    )
    .unwrap_err();

    assert!(matches!(err, QueryError::ContentFilter));
}

#[test]
fn answerable_one_source_query_invokes_provider_and_succeeds() {
    let dir = single_leaf_query_tree();
    let provider = FlakyQueryProvider::new(0, FinishReason::Stop);

    let result = run_with_provider_and_policy(
        dir.path(),
        "what is rust safety",
        &provider,
        &test_model(),
        short_query_policy(1),
    )
    .unwrap();

    assert_eq!(provider.calls(), 1);
    assert_eq!(result.citations.len(), 1);
    assert_eq!(result.citations[0].slug.as_str(), "only-leaf");
}

// ── context assembly tests ───────────────────────────────────────────

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

// ── insufficient sources (zero-citation) tests ───────────────────────────────

struct ZeroCitationProvider;

#[async_trait]
impl LlmProvider for ZeroCitationProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&NormalizedSchema>,
        _reasoning_disabled: bool,
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
        &test_model(),
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
        &test_model(),
        short_query_policy(1),
    )
    .unwrap();

    assert_eq!(result.citations.len(), 1);
    assert_eq!(result.citations[0].slug.as_str(), "only-leaf");
}
