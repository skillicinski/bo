use super::*;
use async_trait::async_trait;
use bo::cli::json as json_output;
use bo::cli::synthesize::{SynthesisError, SynthesisResult};
use bo::domain::tree::TreeConfig;
use bo::domain::{Slug, Timestamp, Title, Url};
use bo::engine::llm::LlmProvider;
use std::cell::Cell;
use std::fs;
use std::path::Path;
use std::sync::Mutex;
use tempfile::TempDir;

#[test]
fn raw_json_mode_detection_stops_at_arg_terminator() {
    assert!(raw_json_mode_requested(&[
        OsString::from("bo"),
        OsString::from("list"),
        OsString::from("--json"),
    ]));
    assert!(!raw_json_mode_requested(&[
        OsString::from("bo"),
        OsString::from("list"),
        OsString::from("--"),
        OsString::from("--json"),
    ]));
}

#[test]
fn synthesize_flags_parse() {
    let cli = Cli::try_parse_from(["bo", "synthesize", "--all"]).unwrap();

    match cli.command {
        Commands::Synthesize { all, .. } => {
            assert!(all);
        }
        other => panic!("expected synthesize command, got {other:?}"),
    }
}

#[test]
fn synthesize_flags_default_false() {
    let cli = Cli::try_parse_from(["bo", "synthesize"]).unwrap();

    match cli.command {
        Commands::Synthesize { all, .. } => {
            assert!(!all);
        }
        other => panic!("expected synthesize command, got {other:?}"),
    }
}

#[test]
fn compile_command_is_rejected() {
    // The old `bo compile` command was renamed to `bo synthesize`; clap must
    // reject the old name with no alias.
    let err = Cli::try_parse_from(["bo", "compile"]).unwrap_err();
    assert_eq!(err.kind(), clap::error::ErrorKind::InvalidSubcommand);
}

#[test]
fn compile_model_flag_is_rejected() {
    // The old `--compile-model` flag was renamed to `--synthesis-model`; clap
    // must reject the old flag with no alias.
    let err = Cli::try_parse_from(["bo", "config", "--compile-model", "gpt-4.1"]).unwrap_err();
    assert!(matches!(
        err.kind(),
        clap::error::ErrorKind::UnknownArgument | clap::error::ErrorKind::InvalidSubcommand
    ));
}

#[test]
fn synthesize_noop_human_output_is_exact_message() {
    let result = SynthesisResult {
        status: "noop".to_string(),
        reason: Some("no new leaves since last synthesis".to_string()),
        mode: None,
        model: None,
        branches: Vec::new(),
        leaves_processed: 0,
        leaves_skipped: Vec::new(),
        notifications: Vec::new(),
        warnings: Vec::new(),
    };
    let mut stdout = Vec::new();

    synthesize::render_human(&result, &mut stdout, "test-tree").unwrap();

    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "nothing new to synthesize\n"
    );
}

#[test]
fn synthesize_noop_json_data_contains_reason() {
    let result = SynthesisResult {
        status: "noop".to_string(),
        reason: Some("no new leaves since last synthesis".to_string()),
        mode: None,
        model: None,
        branches: Vec::new(),
        leaves_processed: 0,
        leaves_skipped: Vec::new(),
        notifications: Vec::new(),
        warnings: Vec::new(),
    };
    let encoded = json_output::success_string("synthesize", &result, Vec::new()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(parsed["data"]["status"], "noop");
    assert_eq!(
        parsed["data"]["reason"],
        "no new leaves since last synthesis"
    );
}

#[test]
fn synthesize_context_overflow_json_recommends_synthesis_model() {
    let error = SynthesisError::ContextOverflow {
        model: "gpt-4o-mini".to_string(),
        estimated_tokens: Some(250_000),
        context_tokens: Some(128_000),
    }
    .json_error();

    assert_eq!(error.code, "context_overflow");
    assert_eq!(error.details["model"], "gpt-4o-mini");
    assert_eq!(error.details["estimated_tokens"], 250_000);
    assert_eq!(error.details["context_tokens"], 128_000);
    assert!(error
        .message
        .contains("bo config --synthesis-model gpt-4.1-mini"));
    assert!(error.details["next_steps"]
        .as_array()
        .unwrap()
        .iter()
        .any(|step| step == "bo config --synthesis-model gpt-4.1"));
}

#[test]
fn synthesize_validation_json_error_includes_next_action() {
    let error = SynthesisError::Validation("invalid synthesis response".to_string()).json_error();

    assert_eq!(error.code, "validation_error");
    assert_eq!(error.message, "invalid synthesis response");
    assert_eq!(error.details["phase"], "synthesis_validation");
    assert_eq!(error.details["files_changed"], false);
    assert_eq!(error.details["next_step"], synthesize::VALIDATION_NEXT_STEP);
}

#[test]
fn query_json_error_includes_low_relevance_details() {
    let error = query::QueryError::LowRelevance {
        reason: query::LowRelevanceReason::GenericTerms,
        matched_sources: 8,
    }
    .json_error();

    assert_eq!(error.code, "low_relevance");
    assert_eq!(error.details["reason"], "generic_query");
    assert_eq!(error.details["matched_sources"], 8);
    assert!(error.details["next_step"]
        .as_str()
        .unwrap()
        .contains("specific"));
}

#[test]
fn query_json_no_answer_errors_include_next_steps() {
    let errors = vec![
        query::QueryError::EmptyTree,
        query::QueryError::NoResults,
        query::QueryError::InsufficientSources {
            leaves_consulted: 2,
        },
    ];

    for error in errors {
        let json_error = error.json_error();
        assert_eq!(json_error.code, error.code());
        assert!(
            json_error.details["next_step"].is_string(),
            "missing next_step for {error:?}"
        );
    }
}

#[test]
fn query_preflight_no_answer_takes_precedence_over_missing_provider() {
    let empty = TempDir::new().unwrap();
    write_state(empty.path(), &[]);
    assert_no_provider_resolver_not_called(&seeded_config(empty.path()), "what is rust", |err| {
        matches!(err, query::QueryError::EmptyTree)
    });

    let no_results = TempDir::new().unwrap();
    write_leaf(
        no_results.path(),
        "cooking.md",
        "Cooking Tips",
        "Boil water and add salt.",
    );
    write_state(
        no_results.path(),
        &[(
            "leaf/cooking.md",
            "Cooking Tips",
            "https://example.com/cooking",
        )],
    );
    assert_no_provider_resolver_not_called(&seeded_config(no_results.path()), "rust", |err| {
        matches!(err, query::QueryError::NoResults)
    });

    // With token-level scoring, "rust" no longer matches inside "Trust";
    // the query correctly returns NoResults (preflight, no provider call).
    let weak = TempDir::new().unwrap();
    write_leaf(
        weak.path(),
        "trust.md",
        "Trust Building",
        "Trust grows slowly.",
    );
    write_state(
        weak.path(),
        &[(
            "leaf/trust.md",
            "Trust Building",
            "https://example.com/trust",
        )],
    );
    assert_no_provider_resolver_not_called(&seeded_config(weak.path()), "rust", |err| {
        matches!(err, query::QueryError::NoResults)
    });
}

#[test]
fn query_relevant_sources_require_provider() {
    let dir = TempDir::new().unwrap();
    write_leaf(
        dir.path(),
        "only-leaf.md",
        "Only Leaf",
        "Rust is a language focused on safety.",
    );
    write_state(
        dir.path(),
        &[("leaf/only-leaf.md", "Only Leaf", "https://example.com/only")],
    );
    let calls = Cell::new(0);

    let err = query::run_with_provider_resolver(
        &seeded_config(dir.path()),
        "what is rust safety",
        || {
            calls.set(calls.get() + 1);
            Err(query::QueryError::NoProvider(
                "missing provider".to_string(),
            ))
        },
    )
    .unwrap_err();

    assert!(matches!(err, query::QueryError::NoProvider(_)));
    assert_eq!(calls.get(), 1);
}

#[test]
fn query_uses_model_not_synthesis_model() {
    let dir = TempDir::new().unwrap();
    write_leaf(
        dir.path(),
        "only-leaf.md",
        "Only Leaf",
        "Rust is a language focused on safety.",
    );
    write_state(
        dir.path(),
        &[("leaf/only-leaf.md", "Only Leaf", "https://example.com/only")],
    );

    let provider = QueryModelRecordingProvider::new();
    let cfg = SeededConfig::new(
        bo::engine::config::Config {
            provider: bo::engine::llm::Provider::OpenAI,
            model: "gpt-4o-mini".to_string(),
            synthesis_model: Some("gpt-4.1".to_string()),
            base_url: None,
            tree: None,
        },
        TreeConfig {
            path: dir.path().to_path_buf(),
            name: "test-tree".to_string(),
            created_at: Timestamp::parse("2026-05-17T00:00:00Z").unwrap(),
        },
    );

    let result = query::run_with_provider_resolver(&cfg, "what is rust safety", || {
        Ok(Box::new(provider.clone_box()) as Box<dyn LlmProvider>)
    });

    assert!(result.is_ok(), "query failed: {result:?}");
    assert_eq!(provider.model().as_deref(), Some("gpt-4o-mini"));
}

struct QueryModelRecordingProvider {
    model: std::sync::Arc<Mutex<Option<String>>>,
}

impl QueryModelRecordingProvider {
    fn new() -> Self {
        Self {
            model: std::sync::Arc::new(Mutex::new(None)),
        }
    }

    fn clone_box(&self) -> Self {
        Self {
            model: std::sync::Arc::clone(&self.model),
        }
    }

    fn model(&self) -> Option<String> {
        self.model.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmProvider for QueryModelRecordingProvider {
    async fn complete(
        &self,
        _messages: &[bo::engine::llm::Message],
        model: &str,
        _max_tokens: u32,
        _response_schema: Option<&bo::engine::llm::ProviderSchema>,
        _reasoning_disabled: bool,
    ) -> Result<bo::engine::llm::LlmResponse, bo::engine::llm::LlmError> {
        *self.model.lock().unwrap() = Some(model.to_string());
        Ok(bo::engine::llm::LlmResponse {
            content: serde_json::json!({
                "answer": "Rust focuses on safety [[only-leaf]].",
                "cited_slugs": ["only-leaf"]
            })
            .to_string(),
            finish_reason: bo::engine::llm::FinishReason::Stop,
        })
    }
}

fn assert_no_provider_resolver_not_called(
    cfg: &SeededConfig,
    question: &str,
    matches_expected_error: impl FnOnce(query::QueryError) -> bool,
) {
    let calls = Cell::new(0);
    let err = query::run_with_provider_resolver(cfg, question, || {
        calls.set(calls.get() + 1);
        Err(query::QueryError::NoProvider(
            "missing provider".to_string(),
        ))
    })
    .unwrap_err();

    assert!(matches_expected_error(err));
    assert_eq!(calls.get(), 0);
}

fn seeded_config(tree: &Path) -> SeededConfig {
    SeededConfig::new(
        bo::engine::config::Config {
            provider: bo::engine::llm::Provider::OpenAI,
            model: "gpt-4o".to_string(),
            synthesis_model: None,
            base_url: None,
            tree: None,
        },
        TreeConfig {
            path: tree.to_path_buf(),
            name: "test-tree".to_string(),
            created_at: Timestamp::parse("2026-05-17T00:00:00Z").unwrap(),
        },
    )
}

fn write_leaf(tree: &Path, filename: &str, title: &str, body: &str) {
    let leaf_dir = tree.join("leaf");
    fs::create_dir_all(&leaf_dir).unwrap();
    fs::write(
        leaf_dir.join(filename),
        format!(
            "---\ntitle: \"{}\"\nurl: \"https://example.com/{}\"\ncollected_at: 2026-01-01T00:00:00Z\n---\n\n{}\n",
            title, filename, body
        ),
    )
    .unwrap();
}

fn write_state(tree: &Path, entries: &[(&str, &str, &str)]) {
    let bo_dir = tree.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    let leaves = entries
        .iter()
        .map(|(file, title, url)| bo::domain::Leaf {
            slug: Slug::generate(&Path::new(file).file_stem().unwrap().to_string_lossy(), ""),
            file: file.to_string(),
            title: Title::parse(title).ok(),
            url: Url::parse(url).unwrap(),
            collected_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            summary: Some(title.to_string()),
        })
        .collect();
    bo::engine::state::write(
        &bo_dir.join("state.json"),
        &bo::domain::state::TreeState {
            tree: bo::domain::state::TreeMetadata {
                name: "test-tree".to_string(),
                created_at: Timestamp::parse("2026-05-17T00:00:00Z").unwrap(),
                last_synthesized_at: None,
            },
            leaves,
            branches: Vec::new(),
        },
    )
    .unwrap();
}
