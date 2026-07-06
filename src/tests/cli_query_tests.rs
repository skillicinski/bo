use super::*;
use crate::domain::{Slug, Timestamp};
use crate::engine::llm::{
    model::Model, FinishReason, LlmProvider, LlmResponse, NormalizedSchema, Provider,
};
use async_trait::async_trait;
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tempfile::TempDir;

fn test_model() -> Model {
    Model::parse("gpt-4o", Provider::OpenAI).unwrap()
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

// ── on-disk tree helpers (for provider-backed synthesis tests) ────────

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
            crate::domain::Leaf {
                slug: Slug::generate(&Path::new(file).file_stem().unwrap().to_string_lossy(), ""),
                file: file.to_string(),
                title: crate::domain::Title::parse(title).ok(),
                url: crate::domain::Url::parse(url).unwrap(),
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
    .unwrap()
}

// ── provider doubles ─────────────────────────────────────────────────

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

// ── synthesis / provider tests ───────────────────────────────────────

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

#[test]
fn unknown_model_rejected_at_parse_boundary() {
    // Model validation now happens at parse time (config boundary),
    // not at query execution time. An invalid model string cannot reach
    // the query pipeline as a &Model.
    assert!(Model::parse("unknown-model", Provider::OpenAI).is_err());
}
