use super::*;
use async_trait::async_trait;
use serde_json::Value;
use serial_test::serial;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tempfile::TempDir;

use crate::engine::auth::MISSING_OPENAI_AUTH_MESSAGE;
use crate::engine::config::SeededConfig;
use crate::engine::llm::{LlmProvider, LlmResponse};

fn make_test_config(output_dir: &std::path::Path) -> SeededConfig {
    SeededConfig {
        tree: crate::domain::tree::TreeConfig {
            output_dir: output_dir.to_path_buf(),
            name: None,
            created_at: None,
        },
        model: Some("gpt-4o-mini".to_string()),
    }
}

struct EnvGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, original }
    }

    fn unset(key: &'static str) -> Self {
        let original = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, original }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

// ── guard tests (ported) ──────────────────────────────────────────────────

#[test]
#[serial]
fn compile_exits_cleanly_on_empty_collection() {
    let dir = TempDir::new().unwrap();
    let cfg = make_test_config(dir.path());
    std::env::remove_var("OPENAI_API_KEY");
    fs::write(dir.path().join("index.jsonl"), "").unwrap();
    let result = cmd_compile(&cfg);
    assert!(result.is_ok());
}

#[test]
#[serial]
fn compile_exits_cleanly_on_single_leaf() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("index.jsonl"),
        r#"{"file":"only.md","title":"Only","url":"https://example.com"}"#,
    )
    .unwrap();
    std::env::remove_var("OPENAI_API_KEY");
    let cfg = make_test_config(dir.path());
    let result = cmd_compile(&cfg);
    assert!(result.is_ok());
}

#[test]
#[serial]
fn compile_errors_without_api_key() {
    let dir = TempDir::new().unwrap();
    let index_path = dir.path().join("index.jsonl");
    // Write two valid leaves so we pass the guard
    fs::write(
        &index_path,
        r#"{"file":"a.md","title":"A","url":"https://example.com/a"}
{"file":"b.md","title":"B","url":"https://example.com/b"}"#,
    )
    .unwrap();
    // Write actual leaf files with valid frontmatter
    fs::write(
        dir.path().join("a.md"),
        "---\ntitle: A\nurl: https://example.com/a\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# A\n\nBody A.\n",
    ).unwrap();
    fs::write(
        dir.path().join("b.md"),
        "---\ntitle: B\nurl: https://example.com/b\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# B\n\nBody B.\n",
    ).unwrap();
    let home = TempDir::new().unwrap();
    let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
    let _api_key_guard = EnvGuard::unset("OPENAI_API_KEY");
    let cfg = make_test_config(dir.path());
    let result = cmd_compile(&cfg);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert_eq!(msg, MISSING_OPENAI_AUTH_MESSAGE);
}

// ── parse_and_validate tests ──────────────────────────────────────────────

fn sample_valid_filenames() -> HashSet<String> {
    ["leaf-a.md", "leaf-b.md", "leaf-c.md"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[test]
fn parse_valid_response() {
    let json = serde_json::json!({
        "branches": [
            {
                "title": "Test Concept",
                "body": "# Test Concept\n\nDescription.",
                "leaves": ["leaf-a.md", "leaf-b.md"]
            }
        ]
    })
    .to_string();
    let json = &json;

    let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
    assert_eq!(plan.branches.len(), 1);
    assert_eq!(plan.branches[0].slug, "test-concept");
    assert_eq!(plan.branches[0].leaves.len(), 2);
    assert_eq!(
        plan.leaf_assignments.get("leaf-a.md").unwrap(),
        &vec!["test-concept".to_string()]
    );
}

#[test]
fn parse_empty_branches_is_valid() {
    let json = r#"{"branches": []}"#;
    let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
    assert!(plan.branches.is_empty());
    assert!(plan.leaf_assignments.is_empty());
}

#[test]
fn validation_error_display_includes_actionable_hint() {
    let message = CompileError::Validation("invalid compile response".to_string()).to_string();
    assert!(message.contains("invalid compile response"));
    assert!(message.contains(VALIDATION_NEXT_STEP));
}

#[test]
fn parse_rejects_missing_required_fields() {
    let json = r##"{"branches":[{"title":"Concept","body":"# Concept\n\nBody."}]}"##;

    let err = parse_and_validate(json, &sample_valid_filenames()).unwrap_err();
    assert!(err.to_string().contains("invalid compile response shape"));
}

#[test]
fn parse_rejects_wrong_field_types() {
    let json =
        r##"{"branches":[{"title":"Concept","body":"# Concept\n\nBody.","leaves":"leaf-a.md"}]}"##;

    let err = parse_and_validate(json, &sample_valid_filenames()).unwrap_err();
    assert!(err.to_string().contains("invalid compile response shape"));
}

#[test]
fn parse_rejects_extra_fields() {
    let json = serde_json::json!({
        "branches": [
            {
                "title": "Concept",
                "body": "# Concept\n\nBody.",
                "leaves": ["leaf-a.md", "leaf-b.md"],
                "confidence": 0.9
            }
        ]
    })
    .to_string();
    let json = &json;

    let err = parse_and_validate(json, &sample_valid_filenames()).unwrap_err();
    assert!(err.to_string().contains("invalid compile response shape"));
}

#[test]
fn parse_rejects_absurdly_large_branch_bodies() {
    let json = serde_json::json!({
        "branches": [
            {
                "title": "Concept",
                "body": "x".repeat(MAX_COMPILED_BODY_BYTES_MIN + 1),
                "leaves": ["leaf-a.md", "leaf-b.md"]
            }
        ]
    })
    .to_string();
    let json = &json;

    let err = parse_and_validate_with_input_size(json, &sample_valid_filenames(), 1).unwrap_err();
    assert!(err.to_string().contains("exceeding"));
}

#[test]
fn parse_rejects_unknown_leaves() {
    let json = serde_json::json!({
        "branches": [
            {
                "title": "Concept",
                "body": "# Concept\n\nBody.",
                "leaves": ["leaf-a.md", "leaf-b.md", "invented.md"]
            }
        ]
    })
    .to_string();
    let json = &json;

    let err = parse_and_validate(json, &sample_valid_filenames()).unwrap_err();
    assert!(err
        .to_string()
        .contains("references unknown leaf 'invented.md'"));
}

#[test]
fn parse_deduplicates_leaves_within_branch() {
    let json = serde_json::json!({
        "branches": [
            {
                "title": "Concept",
                "body": "# Concept\n\nBody.",
                "leaves": ["leaf-a.md", "leaf-a.md", "leaf-b.md"]
            }
        ]
    })
    .to_string();
    let json = &json;

    let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
    assert_eq!(plan.branches[0].leaves, vec!["leaf-a.md", "leaf-b.md"]);
}

#[test]
fn parse_rejects_duplicate_slugs() {
    // "Rust Ownership" and "Rust: Ownership" both slugify to "rust-ownership"
    let json = serde_json::json!({
        "branches": [
            {
                "title": "Rust Ownership",
                "body": "# Rust Ownership\n\nBody.",
                "leaves": ["leaf-a.md", "leaf-b.md"]
            },
            {
                "title": "Rust: Ownership",
                "body": "# Rust: Ownership\n\nBody.",
                "leaves": ["leaf-b.md", "leaf-c.md"]
            }
        ]
    })
    .to_string();
    let json = &json;

    let result = parse_and_validate(json, &sample_valid_filenames());
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("duplicate branch slug"));
}

#[test]
fn parse_rejects_branch_with_all_unknown_leaves() {
    let json = serde_json::json!({
        "branches": [
            {
                "title": "Concept",
                "body": "# Concept\n\nBody.",
                "leaves": ["nonexistent.md"]
            }
        ]
    })
    .to_string();
    let json = &json;

    let err = parse_and_validate(json, &sample_valid_filenames()).unwrap_err();
    assert!(err
        .to_string()
        .contains("references unknown leaf 'nonexistent.md'"));
}

#[test]
fn parse_rejects_branch_with_single_leaf() {
    let json = serde_json::json!({
        "branches": [
            {
                "title": "Solo Concept",
                "body": "# Solo Concept\n\nBody.",
                "leaves": ["leaf-a.md"]
            }
        ]
    })
    .to_string();
    let json = &json;

    let err = parse_and_validate(json, &sample_valid_filenames()).unwrap_err();
    assert!(err
        .to_string()
        .contains("branches must reference at least 2 leaves"));
}

#[test]
fn parse_rejects_branch_with_empty_title() {
    let json = serde_json::json!({
        "branches": [
            {
                "title": "",
                "body": "# Something\n\nBody.",
                "leaves": ["leaf-a.md"]
            }
        ]
    })
    .to_string();
    let json = &json;

    let err = parse_and_validate(json, &sample_valid_filenames()).unwrap_err();
    assert!(err.to_string().contains("empty title"));
}

#[test]
fn parse_rejects_branch_with_empty_body() {
    let json = serde_json::json!({
        "branches": [
            {
                "title": "Concept",
                "body": "",
                "leaves": ["leaf-a.md", "leaf-b.md"]
            }
        ]
    })
    .to_string();
    let json = &json;

    let err = parse_and_validate(json, &sample_valid_filenames()).unwrap_err();
    assert!(err.to_string().contains("empty body"));
}

#[test]
fn validation_failure_does_not_mutate_tree() {
    let dir = TempDir::new().unwrap();
    let cfg = make_test_config(dir.path());

    let leaf_a_path = dir.path().join("leaf-a.md");
    let leaf_b_path = dir.path().join("leaf-b.md");
    let leaf_a_before = "---\ntitle: A\nurl: https://example.com/a\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# A\n\nBody A.\n";
    let leaf_b_before = "---\ntitle: B\nurl: https://example.com/b\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# B\n\nBody B.\n";
    fs::write(&leaf_a_path, leaf_a_before).unwrap();
    fs::write(&leaf_b_path, leaf_b_before).unwrap();

    let valid_filenames: HashSet<String> = ["leaf-a.md", "leaf-b.md"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let bad_response = serde_json::json!({
        "branches": [
            {
                "title": "Concept",
                "body": "# Concept\n\nBody.",
                "leaves": ["leaf-a.md", "invented.md"]
            }
        ]
    })
    .to_string();

    let result = validate_and_execute_plan(
        &bad_response,
        &cfg,
        &valid_filenames,
        leaf_a_before.len() + leaf_b_before.len(),
        "2025-06-01T12:00:00Z",
        &[],
    );

    assert!(matches!(result, Err(CompileError::Validation(_))));
    assert!(!dir.path().join("branches").exists());
    assert_eq!(fs::read_to_string(&leaf_a_path).unwrap(), leaf_a_before);
    assert_eq!(fs::read_to_string(&leaf_b_path).unwrap(), leaf_b_before);
}

// ── execute_plan tests ────────────────────────────────────────────────────

#[test]
fn execute_plan_writes_branches_and_updates_frontmatter() {
    let dir = TempDir::new().unwrap();
    let cfg = make_test_config(dir.path());

    // Write leaf files
    fs::write(
        dir.path().join("leaf-a.md"),
        "---\ntitle: A\nurl: https://example.com/a\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# A\n\nBody.\n",
    ).unwrap();
    fs::write(
        dir.path().join("leaf-b.md"),
        "---\ntitle: B\nurl: https://example.com/b\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# B\n\nBody.\n",
    ).unwrap();

    let valid_filenames: HashSet<String> = ["leaf-a.md", "leaf-b.md"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let mut leaf_assignments = HashMap::new();
    leaf_assignments.insert("leaf-a.md".to_string(), vec!["test-concept".to_string()]);
    leaf_assignments.insert("leaf-b.md".to_string(), vec!["test-concept".to_string()]);

    let plan = CompilePlan {
        branches: vec![ValidatedBranch {
            slug: "test-concept".to_string(),
            title: "Test Concept".to_string(),
            body: "# Test Concept\n\nDescription.\n".to_string(),
            leaves: vec!["leaf-a.md".to_string(), "leaf-b.md".to_string()],
        }],
        leaf_assignments,
    };

    let summary = execute_plan(&plan, &cfg, &valid_filenames, "2025-06-01T12:00:00Z", &[]).unwrap();

    // Branch file written
    let branch_path = dir.path().join("branches").join("test-concept.md");
    assert!(branch_path.exists());
    let branch_content = fs::read_to_string(&branch_path).unwrap();
    assert!(branch_content.contains("title: Test Concept"));
    assert!(branch_content.contains("leaf-a.md"));
    assert!(branch_content.contains("leaf-b.md"));

    // Leaf frontmatter updated
    let leaf_a = fs::read_to_string(dir.path().join("leaf-a.md")).unwrap();
    assert!(leaf_a.contains("branches:"));
    assert!(leaf_a.contains("- test-concept"));
    assert!(leaf_a.contains("updated_at: 2025-06-01T12:00:00Z"));

    // Summary correct
    assert_eq!(summary.branches.len(), 1);
    assert_eq!(summary.leaves_updated, 2);
}

#[test]
fn execute_plan_empty_branches_resets_leaf_frontmatter() {
    let dir = TempDir::new().unwrap();
    let cfg = make_test_config(dir.path());

    // Write a leaf that already has branches assigned
    fs::write(
        dir.path().join("leaf-a.md"),
        "---\ntitle: A\nurl: https://example.com/a\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\nbranches:\n  - old-branch\n---\n\n# A\n\nBody.\n",
    ).unwrap();

    let valid_filenames: HashSet<String> = ["leaf-a.md"].iter().map(|s| s.to_string()).collect();

    let plan = CompilePlan {
        branches: Vec::new(),
        leaf_assignments: HashMap::new(),
    };

    execute_plan(&plan, &cfg, &valid_filenames, "2025-06-01T12:00:00Z", &[]).unwrap();

    let content = fs::read_to_string(dir.path().join("leaf-a.md")).unwrap();
    assert!(content.contains("branches: []"));
}

// ── build_user_message test ───────────────────────────────────────────────

#[test]
fn build_user_message_uses_xml_fencing() {
    let leaves = vec![LoadedLeaf {
        filename: "test.md".to_string(),
        title: "Test Doc".to_string(),
        body: "Some body content.".to_string(),
    }];

    let msg = build_user_message(&leaves);
    assert!(msg.contains("<document filename=\"test.md\" title=\"Test Doc\">"));
    assert!(msg.contains("Some body content."));
    assert!(msg.contains("</document>"));
}

// ── compile_response_schema test ──────────────────────────────────────────

#[test]
fn schema_is_valid_json_schema() {
    let schema = compile_response_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]["branches"].is_object());
    assert_eq!(schema["properties"]["branches"]["type"], "array");
}

// ── LLM policy tests ─────────────────────────────────────────────────────

struct CompileFakeProvider {
    calls: AtomicUsize,
    fail_attempts: usize,
    finish_reason: FinishReason,
}

impl CompileFakeProvider {
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
impl LlmProvider for CompileFakeProvider {
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
            content: r#"{"branches":[]}"#.to_string(),
            finish_reason: self.finish_reason.clone(),
        })
    }
}

struct CompileModelRecordingProvider {
    model: Mutex<Option<String>>,
}

impl CompileModelRecordingProvider {
    fn new() -> Self {
        Self {
            model: Mutex::new(None),
        }
    }

    fn model(&self) -> Option<String> {
        self.model.lock().unwrap().clone()
    }
}

#[async_trait]
impl LlmProvider for CompileModelRecordingProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        model: &str,
        _max_tokens: u32,
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        *self.model.lock().unwrap() = Some(model.to_string());
        Ok(LlmResponse {
            content: r#"{"branches":[]}"#.to_string(),
            finish_reason: FinishReason::Stop,
        })
    }
}

struct CompilePermanentFailureProvider {
    calls: AtomicUsize,
}

impl CompilePermanentFailureProvider {
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
impl LlmProvider for CompilePermanentFailureProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(LlmError::Parse("invalid".to_string()))
    }
}

struct CompileHangingProvider {
    calls: AtomicUsize,
}

impl CompileHangingProvider {
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
impl LlmProvider for CompileHangingProvider {
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
            content: r#"{"branches":[]}"#.to_string(),
            finish_reason: FinishReason::Stop,
        })
    }
}

fn short_compile_policy(max_attempts: usize) -> LlmCallPolicy {
    LlmCallPolicy {
        timeout: Duration::from_millis(20),
        max_attempts,
        initial_backoff: Duration::ZERO,
    }
}

#[tokio::test(flavor = "current_thread")]
async fn compile_retries_transient_failure_and_succeeds() {
    let provider = CompileFakeProvider::new(1, FinishReason::Stop);
    let schema = compile_response_schema();

    let response = call_llm_with_provider(
        &provider,
        "gpt-4o",
        "compile this",
        &schema,
        short_compile_policy(3),
    )
    .await
    .unwrap();

    assert_eq!(provider.calls(), 2);
    assert_eq!(response, r#"{"branches":[]}"#);
}

#[tokio::test(flavor = "current_thread")]
async fn compile_provider_receives_requested_model() {
    let provider = CompileModelRecordingProvider::new();
    let schema = compile_response_schema();

    call_llm_with_provider(
        &provider,
        "gpt-4.1-mini",
        "compile this",
        &schema,
        short_compile_policy(1),
    )
    .await
    .unwrap();

    assert_eq!(provider.model().as_deref(), Some("gpt-4.1-mini"));
}

#[tokio::test(flavor = "current_thread")]
async fn compile_timeout_fails() {
    let provider = CompileHangingProvider::new();
    let schema = compile_response_schema();

    let err = call_llm_with_provider(
        &provider,
        "gpt-4o",
        "compile this",
        &schema,
        short_compile_policy(1),
    )
    .await
    .unwrap_err();

    assert_eq!(provider.calls(), 1);
    assert!(matches!(err, CompileError::Llm(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn compile_does_not_retry_permanent_failure() {
    let provider = CompilePermanentFailureProvider::new();
    let schema = compile_response_schema();

    let err = call_llm_with_provider(
        &provider,
        "gpt-4o",
        "compile this",
        &schema,
        short_compile_policy(3),
    )
    .await
    .unwrap_err();

    assert_eq!(provider.calls(), 1);
    assert!(matches!(err, CompileError::Llm(_)));
}

#[tokio::test(flavor = "current_thread")]
async fn compile_length_finish_reason_returns_truncated() {
    let provider = CompileFakeProvider::new(0, FinishReason::Length);
    let schema = compile_response_schema();

    let err = call_llm_with_provider(
        &provider,
        "gpt-4o",
        "compile this",
        &schema,
        short_compile_policy(1),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, CompileError::Truncated));
}

#[tokio::test(flavor = "current_thread")]
async fn compile_content_filter_finish_reason_returns_content_filter() {
    let provider = CompileFakeProvider::new(0, FinishReason::ContentFilter);
    let schema = compile_response_schema();

    let err = call_llm_with_provider(
        &provider,
        "gpt-4o",
        "compile this",
        &schema,
        short_compile_policy(1),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, CompileError::ContentFilter));
}
