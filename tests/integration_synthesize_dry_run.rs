// Integration tests for `bo synthesize --dry-run`.
//
// Covers the zero-write dry-run surface: empty-tree/noop paths, transaction/repair
// block, agent-without-dry-run rejection, and scripted-provider agent and
// one-shot previews with validation feedback.

mod common;

use bo::cli::synthesize;
use bo::domain::Timestamp;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use tempfile::TempDir;

use common::bo;

// ── subprocess helpers ──────────────────────────────────────────────────────

fn seed(home: &Path, name: &str) -> PathBuf {
    common::seed_with(home, name, "deepseek", "deepseek-v4-flash")
}

fn synthesize_cmd(home: &Path, args: &[&str]) -> Output {
    bo(home)
        .arg("synthesize")
        .args(args)
        .output()
        .expect("failed to run bo synthesize")
}

fn make_config(output_dir: &Path) -> bo::engine::config::SeededConfig {
    common::seeded_config(
        output_dir,
        bo::engine::llm::Provider::Deepseek,
        "deepseek-v4-flash",
    )
}

fn synthesis_model() -> bo::engine::llm::Model {
    bo::engine::llm::Model::parse("deepseek-v4-flash", bo::engine::llm::Provider::Deepseek).unwrap()
}

// ── ScriptedProvider: agent mode (complete_with_tools) ──────────────────────

/// A scripted `LlmProvider` whose `complete_with_tools` returns a sequence
/// of pre-built `AgentResponse` values. After the sequence is exhausted,
/// returns an empty text response (no tool calls) as a fallback.
struct ScriptedAgentProvider {
    responses: Vec<bo::engine::llm::AgentResponse>,
    call_count: AtomicUsize,
}

impl ScriptedAgentProvider {
    fn new(responses: Vec<bo::engine::llm::AgentResponse>) -> Self {
        Self {
            responses,
            call_count: AtomicUsize::new(0),
        }
    }

    fn next(&self) -> Option<&bo::engine::llm::AgentResponse> {
        let i = self.call_count.fetch_add(1, Ordering::SeqCst);
        self.responses.get(i)
    }
}

#[async_trait]
impl bo::engine::llm::LlmProvider for ScriptedAgentProvider {
    async fn complete(
        &self,
        _messages: &[bo::engine::llm::Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&bo::engine::llm::ProviderSchema>,
        _reasoning_disabled: bool,
    ) -> Result<bo::engine::llm::LlmResponse, bo::engine::llm::LlmError> {
        // Unreachable for agent tests, but provide a stubbed response.
        Ok(bo::engine::llm::LlmResponse {
            content: "{}".to_string(),
            finish_reason: bo::engine::llm::FinishReason::Stop,
        })
    }

    async fn complete_with_tools(
        &self,
        _messages: &[bo::engine::llm::AgentMessage],
        _model: &str,
        _max_tokens: u32,
        _tools: &[bo::engine::llm::ToolSchema],
        _reasoning_disabled: bool,
    ) -> Result<bo::engine::llm::AgentResponse, bo::engine::llm::LlmError> {
        match self.next() {
            Some(resp) => Ok(resp.clone()),
            None => Ok(bo::engine::llm::AgentResponse {
                content: Some(String::new()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: bo::engine::llm::FinishReason::Stop,
                usage: None,
            }),
        }
    }
}

// ── ScriptedOneShotProvider: one-shot mode (complete) ───────────────────────

struct ScriptedOneShotProvider {
    response: String,
}

#[async_trait]
impl bo::engine::llm::LlmProvider for ScriptedOneShotProvider {
    async fn complete(
        &self,
        _messages: &[bo::engine::llm::Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&bo::engine::llm::ProviderSchema>,
        _reasoning_disabled: bool,
    ) -> Result<bo::engine::llm::LlmResponse, bo::engine::llm::LlmError> {
        Ok(bo::engine::llm::LlmResponse {
            content: self.response.clone(),
            finish_reason: bo::engine::llm::FinishReason::Stop,
        })
    }
}

// ── helpers for building canned AgentResponse values ────────────────────────

fn tool_call(id: &str, name: &str, arguments: &str) -> bo::engine::llm::ToolCall {
    bo::engine::llm::ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: arguments.to_string(),
    }
}

fn tool_response(id: &str, name: &str, arguments: &str) -> bo::engine::llm::AgentResponse {
    tool_response_many(vec![tool_call(id, name, arguments)])
}

fn tool_response_many(calls: Vec<bo::engine::llm::ToolCall>) -> bo::engine::llm::AgentResponse {
    bo::engine::llm::AgentResponse {
        content: None,
        reasoning_content: None,
        tool_calls: calls,
        finish_reason: bo::engine::llm::FinishReason::Other("tool_calls".into()),
        usage: None,
    }
}

// ── tests ───────────────────────────────────────────────────────────────────

#[test]
fn synthesize_no_flags_empty_tree_byte_for_byte() {
    let home = TempDir::new().unwrap();
    seed(home.path(), "test-grove");

    let out = synthesize_cmd(home.path(), &[]);
    assert!(
        out.status.success(),
        "synthesize failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let expected = "test-grove is empty\n";
    assert_eq!(
        stdout,
        expected,
        "stdout byte mismatch.\ngot:      {:?}\nexpected: {:?}",
        stdout.as_bytes(),
        expected.as_bytes()
    );
}

#[test]
fn synthesize_dry_run_empty_tree_is_noop_and_zero_write() {
    let home = TempDir::new().unwrap();
    let tree = seed(home.path(), "meadow");

    let before = common::snapshot_tree(&tree);

    let out = synthesize_cmd(home.path(), &["--dry-run"]);
    assert!(
        out.status.success(),
        "synthesize --dry-run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("is empty"), "stdout: {stdout}");
    assert!(
        stdout.contains("dry run: no files were written"),
        "stdout: {stdout}"
    );

    let after = common::snapshot_tree(&tree);
    assert_eq!(
        before, after,
        "tree was modified by --dry-run on empty tree"
    );
}

#[test]
fn agent_without_dry_run_is_rejected() {
    let home = TempDir::new().unwrap();
    seed(home.path(), "copse");

    let out = synthesize_cmd(home.path(), &["--agent"]);
    assert!(!out.status.success(), "expected non-zero exit code");
    assert_eq!(
        out.status.code(),
        Some(2),
        "expected exit code 2 (clap usage error), got {:?}",
        out.status.code()
    );

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--dry-run"),
        "expected stderr to mention --dry-run, got: {stderr}"
    );
}

#[test]
fn dry_run_blocked_by_unfinished_transaction_writes_zero() {
    let dir = common::setup_fixture_collection();
    let cfg = make_config(dir.path());
    let tree_dir = dir.path();
    let bo_dir = tree_dir.join(".bo");

    let state_hash = bo::engine::transaction::state_hash(tree_dir).unwrap();

    // Write a pending.json with a dead PID (99999) and no staged files
    let transaction = bo::engine::transaction::PendingTransaction {
        op: bo::engine::transaction::TransactionKind::Synthesize {
            mode: bo::engine::transaction::SynthesisMode::Full,
        },
        started_at: "2020-01-01T00:00:00Z".to_string(),
        pid: 99999,
        pre_state_hash: state_hash,
        writes: Vec::new(),
        deletes: Vec::new(),
    };
    let pending_path = bo_dir.join("pending.json");
    bo::engine::transaction::write(&pending_path, &transaction).unwrap();

    let before = common::snapshot_tree(tree_dir);

    let outcome = synthesize::run_dry_run(
        &cfg,
        synthesize::SynthesisOptions {
            all: false,
            agent: false,
            dry_run: true,
        },
    );

    match outcome.result {
        Err(synthesize::SynthesisError::DryRunBlocked(msg)) => {
            assert!(
                msg.contains("unfinished transaction"),
                "expected transaction message, got: {msg}"
            );
        }
        other => panic!("expected DryRunBlocked, got: {other:?}"),
    }

    // pending.json must still exist (not recovered)
    assert!(
        pending_path.exists(),
        "pending.json should still exist after dry-run block"
    );

    // tree must be byte-identical
    let after = common::snapshot_tree(tree_dir);
    assert_eq!(before, after, "tree was modified by blocked dry-run");
}

#[test]
fn dry_run_blocked_by_stale_repair_writes_zero() {
    let dir = common::setup_fixture_collection();
    let cfg = make_config(dir.path());
    let tree_dir = dir.path();
    let bo_dir = tree_dir.join(".bo");

    // Delete one leaf file so repair is required
    let missing = tree_dir.join("rust-ownership.md");
    fs::remove_file(&missing).unwrap();
    assert!(!missing.exists());

    // Set last_synthesized_at so the state has a known-synthesized state
    let mut m = bo::engine::state::read(&bo_dir.join("state.json")).unwrap();
    m.tree.last_synthesized_at = Some(Timestamp::parse("2025-06-02T00:00:00Z").unwrap());
    bo::engine::state::write(&bo_dir.join("state.json"), &m).unwrap();

    let state_hash = bo::engine::transaction::state_hash(tree_dir).unwrap();
    let before = common::snapshot_tree(tree_dir);

    let outcome = synthesize::run_dry_run(
        &cfg,
        synthesize::SynthesisOptions {
            all: false,
            agent: false,
            dry_run: true,
        },
    );

    match outcome.result {
        Err(synthesize::SynthesisError::DryRunBlocked(msg)) => {
            assert!(
                msg.contains("repair"),
                "expected repair message, got: {msg}"
            );
        }
        other => panic!("expected DryRunBlocked, got: {other:?}"),
    }

    // TreeState hash must be unchanged
    let after_hash = bo::engine::transaction::state_hash(tree_dir).unwrap();
    assert_eq!(
        state_hash, after_hash,
        "state hash changed during blocked dry-run"
    );

    // No branch/ files created
    let branch_dir = tree_dir.join("branch");
    assert!(
        !branch_dir.exists() || fs::read_dir(&branch_dir).unwrap().next().is_none(),
        "branch/ directory has files after blocked dry-run"
    );

    let after = common::snapshot_tree(tree_dir);
    assert_eq!(before, after, "tree was modified by blocked dry-run");
}

#[test]
fn agent_dry_run_with_scripted_provider_produces_preview_and_zero_writes() {
    let dir = common::setup_fixture_collection();
    let cfg = make_config(dir.path());
    let model = synthesis_model();
    let tree_dir = dir.path();

    let before = common::snapshot_tree(tree_dir);
    let before_hash = bo::engine::transaction::state_hash(tree_dir).unwrap();

    // Turn 1: list_leaves (no args needed; the tool defaults offset/limit)
    // Turn 2: read_leaf with a real slug
    // Turn 3: submit_synthesis with valid Full plan
    let valid_plan = serde_json::json!({
        "branches": [
            {
                "title": "Rust Memory Safety",
                "body": "# Rust Memory Safety\n\nOwnership and borrowing make memory safety a compile-time guarantee.",
                "leaves": ["rust-ownership.md", "memory-safety.md"]
            },
            {
                "title": "Systems Design in Rust",
                "body": "# Systems Design in Rust\n\nZero-cost abstractions and safe concurrency shape Rust systems code.",
                "leaves": ["safe-concurrency.md", "zero-cost-abstractions.md"]
            }
        ]
    })
    .to_string();

    let script = vec![
        tool_response("call_1", "list_leaves", "{}"),
        tool_response("call_2", "read_leaf", r#"{"slug":"rust-ownership"}"#),
        tool_response("call_3", "submit_synthesis", &valid_plan),
    ];
    let provider = ScriptedAgentProvider::new(script);

    let outcome = synthesize::run_dry_run_with_provider(
        &cfg,
        synthesize::SynthesisOptions {
            all: false,
            agent: true,
            dry_run: true,
        },
        &provider,
        &model,
    );

    let preview = outcome.result.unwrap();
    assert_eq!(preview.status, "preview");
    assert!(preview.agent);
    assert!(
        preview.turns >= 2,
        "expected >=2 turns, got {}",
        preview.turns
    );
    assert!(
        preview.tool_calls >= 2,
        "expected >=2 tool calls, got {}",
        preview.tool_calls
    );
    assert!(!preview.branches.is_empty());
    assert!(preview.state_unchanged);

    let after = common::snapshot_tree(tree_dir);
    assert_eq!(before, after, "tree was modified by agent dry-run");

    let after_hash = bo::engine::transaction::state_hash(tree_dir).unwrap();
    assert_eq!(before_hash, after_hash, "state hash changed");
}

#[test]
fn one_shot_dry_run_with_scripted_provider_produces_preview() {
    let dir = common::setup_fixture_collection();
    let cfg = make_config(dir.path());
    let model = synthesis_model();
    let tree_dir = dir.path();

    let before = common::snapshot_tree(tree_dir);
    let before_hash = bo::engine::transaction::state_hash(tree_dir).unwrap();

    let valid_plan = serde_json::json!({
        "branches": [
            {
                "title": "Rust Memory Safety",
                "body": "# Rust Memory Safety\n\nOwnership and borrowing make memory safety a compile-time guarantee.",
                "leaves": ["rust-ownership.md", "memory-safety.md"]
            },
            {
                "title": "Systems Design in Rust",
                "body": "# Systems Design in Rust\n\nZero-cost abstractions and safe concurrency shape Rust systems code.",
                "leaves": ["safe-concurrency.md", "zero-cost-abstractions.md"]
            }
        ]
    })
    .to_string();
    let provider = ScriptedOneShotProvider {
        response: valid_plan,
    };

    let outcome = synthesize::run_dry_run_with_provider(
        &cfg,
        synthesize::SynthesisOptions {
            all: false,
            agent: false,
            dry_run: true,
        },
        &provider,
        &model,
    );

    let preview = outcome.result.unwrap();
    assert_eq!(preview.status, "preview");
    assert!(!preview.agent);
    assert_eq!(preview.turns, 1);
    assert_eq!(preview.tool_calls, 0);
    assert!(!preview.branches.is_empty());

    let after = common::snapshot_tree(tree_dir);
    assert_eq!(before, after, "tree was modified by one-shot dry-run");

    let after_hash = bo::engine::transaction::state_hash(tree_dir).unwrap();
    assert_eq!(before_hash, after_hash, "state hash changed");
}

#[test]
fn agent_dry_run_validation_feedback_then_success() {
    let dir = common::setup_fixture_collection();
    let cfg = make_config(dir.path());
    let model = synthesis_model();
    let tree_dir = dir.path();

    let before = common::snapshot_tree(tree_dir);
    let before_hash = bo::engine::transaction::state_hash(tree_dir).unwrap();

    // Turn 1: invalid submit_synthesis (branch references unknown leaf)
    let invalid_plan = serde_json::json!({
        "branches": [
            {
                "title": "Fake Branch",
                "body": "This branch references a leaf that doesn't exist.",
                "leaves": ["nonexistent.md", "rust-ownership.md"]
            }
        ]
    })
    .to_string();

    // Turn 2: valid plan
    let valid_plan = serde_json::json!({
        "branches": [
            {
                "title": "Rust Memory Safety",
                "body": "# Rust Memory Safety\n\nOwnership and borrowing make memory safety a compile-time guarantee.",
                "leaves": ["rust-ownership.md", "memory-safety.md"]
            },
            {
                "title": "Systems Design in Rust",
                "body": "# Systems Design in Rust\n\nZero-cost abstractions and safe concurrency shape Rust systems code.",
                "leaves": ["safe-concurrency.md", "zero-cost-abstractions.md"]
            }
        ]
    })
    .to_string();

    let script = vec![
        tool_response("call_1", "submit_synthesis", &invalid_plan),
        tool_response("call_2", "submit_synthesis", &valid_plan),
    ];
    let provider = ScriptedAgentProvider::new(script);

    let outcome = synthesize::run_dry_run_with_provider(
        &cfg,
        synthesize::SynthesisOptions {
            all: false,
            agent: true,
            dry_run: true,
        },
        &provider,
        &model,
    );

    let preview = outcome.result.unwrap();
    assert_eq!(preview.status, "preview");
    assert_eq!(preview.turns, 2, "expected 2 turns, got {}", preview.turns);
    assert!(!preview.branches.is_empty());

    let after = common::snapshot_tree(tree_dir);
    assert_eq!(
        before, after,
        "tree was modified by agent validation-feedback dry-run"
    );

    let after_hash = bo::engine::transaction::state_hash(tree_dir).unwrap();
    assert_eq!(before_hash, after_hash, "state hash changed");
}

#[test]
fn agent_dry_run_unsupported_provider_errors_with_actionable_message() {
    // A provider that implements only `complete` (not `complete_with_tools`)
    // uses the trait's default, which returns an explicit unsupported error.
    // The agent path must surface it as an actionable AgentFailed, not panic
    // or silently degrade. Covers acceptance #6 (unsupported providers).
    let dir = common::setup_fixture_collection();
    let cfg = make_config(dir.path());
    let model = synthesis_model();
    let tree_dir = dir.path();

    let before = common::snapshot_tree(tree_dir);

    // ScriptedOneShotProvider implements `complete` only; the default
    // `complete_with_tools` returns the unsupported error.
    let provider = ScriptedOneShotProvider {
        response: "irrelevant".to_string(),
    };

    let outcome = synthesize::run_dry_run_with_provider(
        &cfg,
        synthesize::SynthesisOptions {
            all: false,
            agent: true,
            dry_run: true,
        },
        &provider,
        &model,
    );

    let err = outcome
        .result
        .expect_err("expected an error for an unsupported provider");
    assert!(
        matches!(err, synthesize::SynthesisError::AgentFailed { .. }),
        "expected AgentFailed, got {err:?}"
    );
    let msg = err.to_string();
    assert!(
        msg.contains("does not support tool calls"),
        "expected an actionable unsupported-provider message, got: {msg}"
    );

    // Zero writes even on the error path.
    let after = common::snapshot_tree(tree_dir);
    assert_eq!(
        before, after,
        "tree was modified on the unsupported-provider path"
    );
}

#[test]
fn agent_dry_run_limit_failure_surfaces_diagnostics_in_json() {
    // 8 list_leaves calls per turn × 6 turns = 48 = MAX_TOTAL_TOOL_CALLS. The
    // agent hits the total-tool-call limit. The error envelope must carry the
    // same resource diagnostics as a success envelope: turns, tool_calls,
    // usage, and last_error (the last tool-result error).
    let dir = common::setup_fixture_collection();
    let cfg = make_config(dir.path());
    let model = synthesis_model();
    let tree_dir = dir.path();

    let before = common::snapshot_tree(tree_dir);

    let script: Vec<bo::engine::llm::AgentResponse> = (0..6)
        .map(|t| {
            let calls: Vec<bo::engine::llm::ToolCall> = (0..8)
                .map(|j| tool_call(&format!("c{t}_{j}"), "list_leaves", "{}"))
                .collect();
            tool_response_many(calls)
        })
        .collect();
    let provider = ScriptedAgentProvider::new(script);

    let outcome = synthesize::run_dry_run_with_provider(
        &cfg,
        synthesize::SynthesisOptions {
            all: false,
            agent: true,
            dry_run: true,
        },
        &provider,
        &model,
    );

    let err = outcome.result.expect_err("expected a limit-failure error");
    let json = err.json_error();
    assert_eq!(json.code, "agent_error", "unexpected error code: {json:?}");
    let details = &json.details;
    for key in ["turns", "tool_calls", "usage", "last_error"] {
        assert!(
            details.get(key).is_some(),
            "agent_error details missing `{key}`: {details}"
        );
    }
    assert_eq!(
        details["turns"].as_u64(),
        Some(6),
        "expected 6 turns, got: {details}"
    );
    assert_eq!(
        details["tool_calls"].as_u64(),
        Some(48),
        "expected 48 tool calls, got: {details}"
    );

    let after = common::snapshot_tree(tree_dir);
    assert_eq!(before, after, "limit-failure path wrote bytes");
}
