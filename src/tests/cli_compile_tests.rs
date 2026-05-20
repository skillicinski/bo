use super::*;
use crate::domain::{Slug, Timestamp, Title, Url};
use async_trait::async_trait;
use serde_json::Value;
use serial_test::serial;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tempfile::TempDir;

use crate::domain::manifest::{self, BranchRecord, LeafRecord, Manifest, TreeMeta};
use crate::engine::auth::MISSING_OPENAI_AUTH_MESSAGE;
use crate::engine::config::SeededConfig;
use crate::engine::llm::{LlmError, LlmProvider, LlmResponse, Message};

// ── helpers ───────────────────────────────────────────────────────────────────

fn make_test_config(output_dir: &std::path::Path) -> SeededConfig {
    SeededConfig {
        tree: crate::domain::tree::TreeConfig {
            output_dir: output_dir.to_path_buf(),
            name: None,
            created_at: None,
        },
        model: Some("gpt-4o-mini".to_string()),
        compile_model: None,
    }
}

fn seed_manifest(dir: &std::path::Path, leaves: &[(&str, &str, &str)]) {
    fs::create_dir_all(dir.join(".bo")).unwrap();
    let leaf_records = leaves
        .iter()
        .map(|(slug, title, url)| LeafRecord {
            slug: Slug::parse(slug).unwrap(),
            file: format!("{}.md", slug),
            title: Title::new(title),
            url: Url::parse(url).unwrap(),
            collected_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            summary: None,
        })
        .collect();
    let m = Manifest {
        tree: TreeMeta {
            name: "compile-tree".to_string(),
            created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            last_compiled_at: None,
        },
        leaves: leaf_records,
        branches: Vec::new(),
    };
    manifest::write(&dir.join(".bo/manifest.json"), &m).unwrap();
}

fn write_leaf(dir: &std::path::Path, slug: &str, title: &str, url: &str) {
    fs::write(
        dir.join(format!("{}.md", slug)),
        format!(
            "---\ntitle: \"{title}\"\nurl: {url}\ncollected_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\n---\n\n# {title}\n\nBody for {slug}.\n"
        ),
    )
    .unwrap();
}

fn seed_compiled_tree(dir: &std::path::Path) {
    seed_manifest(
        dir,
        &[
            ("leaf-a", "A", "https://example.com/a"),
            ("leaf-b", "B", "https://example.com/b"),
        ],
    );
    write_leaf(dir, "leaf-a", "A", "https://example.com/a");
    write_leaf(dir, "leaf-b", "B", "https://example.com/b");
    let manifest_path = dir.join(".bo/manifest.json");
    let mut m = manifest::read(&manifest_path).unwrap();
    m.tree.last_compiled_at = Some(Timestamp::parse("2026-06-01T12:00:00Z").unwrap());
    m.branches = vec![BranchRecord {
        slug: Slug::parse("existing").unwrap(),
        file: "branches/existing.md".to_string(),
        title: Title::new("Existing"),
        created_at: Timestamp::parse("2026-06-01T12:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-06-01T12:00:00Z").unwrap(),
        stale: false,
        leaves: vec![
            Slug::parse("leaf-a").unwrap(),
            Slug::parse("leaf-b").unwrap(),
        ],
    }];
    manifest::write(&manifest_path, &m).unwrap();
    fs::create_dir_all(dir.join("branches")).unwrap();
    fs::write(
        dir.join("branches/existing.md"),
        "---\ntitle: Existing\ncreated_at: 2026-06-01T12:00:00Z\nupdated_at: 2026-06-01T12:00:00Z\nleaves:\n  - leaf-a.md\n  - leaf-b.md\n---\n\n# Existing\n\nBranch body.\n",
    )
    .unwrap();
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

// ── fake providers ────────────────────────────────────────────────────────────

struct StaticProvider {
    response: String,
    calls: AtomicUsize,
}

impl StaticProvider {
    fn new(response: serde_json::Value) -> Self {
        Self {
            response: response.to_string(),
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for StaticProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(LlmResponse {
            content: self.response.clone(),
            finish_reason: crate::engine::llm::FinishReason::Stop,
        })
    }
}

struct ModelRecordingProvider {
    model: Mutex<Option<String>>,
}

impl ModelRecordingProvider {
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
impl LlmProvider for ModelRecordingProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        model: &str,
        _max_tokens: u32,
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        *self.model.lock().unwrap() = Some(model.to_string());
        Ok(LlmResponse {
            content: r#"{"updated_branches":[],"new_branches":[]}"#.to_string(),
            finish_reason: crate::engine::llm::FinishReason::Stop,
        })
    }
}

fn empty_incremental_response() -> serde_json::Value {
    serde_json::json!({"updated_branches": [], "new_branches": []})
}

// ── scenario tests ────────────────────────────────────────────────────────────

#[test]
#[serial]
fn compile_exits_cleanly_on_empty_collection() {
    let dir = TempDir::new().unwrap();
    let cfg = make_test_config(dir.path());
    std::env::remove_var("OPENAI_API_KEY");
    seed_manifest(dir.path(), &[]);
    let result = cmd_compile(&cfg);
    assert!(result.is_ok());
}

#[test]
#[serial]
fn compile_exits_cleanly_on_single_leaf() {
    let dir = TempDir::new().unwrap();
    seed_manifest(dir.path(), &[("only", "Only", "https://example.com")]);
    write_leaf(dir.path(), "only", "Only", "https://example.com");
    std::env::remove_var("OPENAI_API_KEY");
    let cfg = make_test_config(dir.path());
    let result = cmd_compile(&cfg);
    assert!(result.is_ok());
}

#[test]
#[serial]
fn compile_errors_without_api_key() {
    let dir = TempDir::new().unwrap();
    seed_manifest(
        dir.path(),
        &[
            ("a", "A", "https://example.com/a"),
            ("b", "B", "https://example.com/b"),
        ],
    );
    write_leaf(dir.path(), "a", "A", "https://example.com/a");
    write_leaf(dir.path(), "b", "B", "https://example.com/b");
    let home = TempDir::new().unwrap();
    let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
    let _api_key_guard = EnvGuard::unset("OPENAI_API_KEY");
    let cfg = make_test_config(dir.path());
    let result = cmd_compile(&cfg);
    assert!(result.is_err());
    let msg = result.unwrap_err();
    assert_eq!(msg, MISSING_OPENAI_AUTH_MESSAGE);
}

// ── first compile creates branches ────────────────────────────────────────────

#[test]
fn first_compile_creates_branches_and_reports_incremental_mode() {
    let dir = TempDir::new().unwrap();
    seed_manifest(
        dir.path(),
        &[
            ("leaf-a", "A", "https://example.com/a"),
            ("leaf-b", "B", "https://example.com/b"),
        ],
    );
    write_leaf(dir.path(), "leaf-a", "A", "https://example.com/a");
    write_leaf(dir.path(), "leaf-b", "B", "https://example.com/b");
    let cfg = make_test_config(dir.path());
    let provider = StaticProvider::new(serde_json::json!({
        "updated_branches": [],
        "new_branches": [{
            "title": "Test Concept",
            "body": "# Test Concept\n\nBody.",
            "leaves": ["leaf-a", "leaf-b"]
        }]
    }));

    let result = run_compile_with_provider(
        &cfg,
        CompileOptions::default(),
        &provider,
        &cfg.effective_compile_model(),
    )
    .unwrap();

    assert_eq!(result.status, "compiled");
    assert_eq!(result.mode, Some(CompileRunMode::Incremental));
    assert_eq!(result.context_mode, Some(CompileContextMode::FullCorpus));
    assert_eq!(provider.calls(), 1);

    // Manifest updated
    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.branches.len(), 1);
    assert_eq!(m.branches[0].slug.as_str(), "test-concept");
    assert_eq!(
        m.branches[0]
            .leaves
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>(),
        vec!["leaf-a", "leaf-b"]
    );
    assert!(m.tree.last_compiled_at.is_some());

    // Branch file written
    assert!(dir.path().join("branches/test-concept.md").exists());

    // Leaf files unchanged
    let leaf_a = fs::read_to_string(dir.path().join("leaf-a.md")).unwrap();
    assert!(leaf_a.contains("Body for leaf-a."));
    assert!(!leaf_a.contains("branches:"));
}

// ── no-op ─────────────────────────────────────────────────────────────────────

#[test]
fn no_op_when_no_new_leaves_and_no_stale_branches() {
    let dir = TempDir::new().unwrap();
    seed_compiled_tree(dir.path());
    let cfg = make_test_config(dir.path());
    let provider = StaticProvider::new(empty_incremental_response());

    let result = run_compile_with_provider(
        &cfg,
        CompileOptions::default(),
        &provider,
        &cfg.effective_compile_model(),
    )
    .unwrap();

    assert_eq!(result.status, "noop");
    assert_eq!(
        result.reason.as_deref(),
        Some("no new leaves since last compile")
    );
    assert_eq!(provider.calls(), 0);
}

// ── compile_model routing ─────────────────────────────────────────────────────

#[test]
fn compile_uses_effective_compile_model() {
    let dir = TempDir::new().unwrap();
    seed_manifest(
        dir.path(),
        &[
            ("leaf-a", "A", "https://example.com/a"),
            ("leaf-b", "B", "https://example.com/b"),
        ],
    );
    write_leaf(dir.path(), "leaf-a", "A", "https://example.com/a");
    write_leaf(dir.path(), "leaf-b", "B", "https://example.com/b");
    let cfg = SeededConfig {
        tree: crate::domain::tree::TreeConfig {
            output_dir: dir.path().to_path_buf(),
            name: None,
            created_at: None,
        },
        model: Some("gpt-4o-mini".to_string()),
        compile_model: Some("gpt-4.1-mini".to_string()),
    };
    let provider = ModelRecordingProvider::new();

    run_compile_with_provider(
        &cfg,
        CompileOptions::default(),
        &provider,
        &cfg.effective_compile_model(),
    )
    .unwrap();

    assert_eq!(provider.model().as_deref(), Some("gpt-4.1-mini"));
}

// ── full mode (--all) deletes omitted branches ────────────────────────────────

#[test]
fn full_mode_deletes_omitted_branch_files() {
    let dir = TempDir::new().unwrap();
    seed_compiled_tree(dir.path());
    // Add a new leaf so compile has work to do
    let manifest_path = dir.path().join(".bo/manifest.json");
    let mut m = manifest::read(&manifest_path).unwrap();
    m.leaves.push(LeafRecord {
        slug: Slug::parse("leaf-c").unwrap(),
        file: "leaf-c.md".to_string(),
        title: Title::new("C"),
        url: Url::parse("https://example.com/c").unwrap(),
        collected_at: Timestamp::parse("2026-07-01T00:00:00Z").unwrap(),
        summary: None,
    });
    manifest::write(&manifest_path, &m).unwrap();
    write_leaf(dir.path(), "leaf-c", "C", "https://example.com/c");

    let cfg = make_test_config(dir.path());
    // Full response replaces graph with a different branch — "existing" is omitted
    let provider = StaticProvider::new(serde_json::json!({
        "branches": [{
            "title": "Replacement",
            "body": "# Replacement\n\nNew graph.",
            "leaves": ["leaf-a.md", "leaf-b.md", "leaf-c.md"]
        }]
    }));

    let result = run_compile_with_provider(
        &cfg,
        CompileOptions { all: true },
        &provider,
        &cfg.effective_compile_model(),
    )
    .unwrap();

    assert_eq!(result.status, "compiled");
    assert_eq!(result.mode, Some(CompileRunMode::Full));

    // Old branch file deleted
    assert!(!dir.path().join("branches/existing.md").exists());
    // New branch file created
    assert!(dir.path().join("branches/replacement.md").exists());

    let m = manifest::read(&manifest_path).unwrap();
    assert!(m.branch_by_slug_str("existing").is_none());
    assert!(m.branch_by_slug_str("replacement").is_some());
}

// ── incremental compile preserves omitted branches ────────────────────────────

#[test]
fn incremental_compile_preserves_existing_branches() {
    let dir = TempDir::new().unwrap();
    seed_compiled_tree(dir.path());
    // Add a new leaf
    let manifest_path = dir.path().join(".bo/manifest.json");
    let mut m = manifest::read(&manifest_path).unwrap();
    m.leaves.push(LeafRecord {
        slug: Slug::parse("leaf-c").unwrap(),
        file: "leaf-c.md".to_string(),
        title: Title::new("C"),
        url: Url::parse("https://example.com/c").unwrap(),
        collected_at: Timestamp::parse("2026-07-01T00:00:00Z").unwrap(),
        summary: None,
    });
    manifest::write(&manifest_path, &m).unwrap();
    write_leaf(dir.path(), "leaf-c", "C", "https://example.com/c");

    let cfg = make_test_config(dir.path());
    // Response creates a new branch but does NOT mention "existing"
    let provider = StaticProvider::new(serde_json::json!({
        "updated_branches": [],
        "new_branches": [{
            "title": "New Concept",
            "body": "# New Concept\n\nBody.",
            "leaves": ["leaf-a", "leaf-c"]
        }]
    }));

    let result = run_compile_with_provider(
        &cfg,
        CompileOptions::default(),
        &provider,
        &cfg.effective_compile_model(),
    )
    .unwrap();

    assert_eq!(result.status, "compiled");
    assert_eq!(result.mode, Some(CompileRunMode::Incremental));

    let m = manifest::read(&manifest_path).unwrap();
    // Existing branch preserved
    assert!(m.branch_by_slug_str("existing").is_some());
    // New branch created
    assert!(m.branch_by_slug_str("new-concept").is_some());
    // Old branch file still exists
    assert!(dir.path().join("branches/existing.md").exists());
    // New branch file written
    assert!(dir.path().join("branches/new-concept.md").exists());
}

// ── stale branch scenarios ────────────────────────────────────────────────

#[test]
fn deleted_leaf_rebuilds_stale_branch() {
    let dir = TempDir::new().unwrap();
    seed_manifest(
        dir.path(),
        &[
            ("leaf-a", "A", "https://example.com/a"),
            ("leaf-b", "B", "https://example.com/b"),
            ("leaf-c", "C", "https://example.com/c"),
        ],
    );
    write_leaf(dir.path(), "leaf-a", "A", "https://example.com/a");
    write_leaf(dir.path(), "leaf-b", "B", "https://example.com/b");
    write_leaf(dir.path(), "leaf-c", "C", "https://example.com/c");
    let manifest_path = dir.path().join(".bo/manifest.json");
    let mut m = manifest::read(&manifest_path).unwrap();
    m.tree.last_compiled_at = Some(Timestamp::parse("2026-06-01T12:00:00Z").unwrap());
    m.branches = vec![BranchRecord {
        slug: Slug::parse("concept").unwrap(),
        file: "branches/concept.md".to_string(),
        title: Title::new("Concept"),
        created_at: Timestamp::parse("2026-06-01T12:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-06-01T12:00:00Z").unwrap(),
        stale: false,
        leaves: vec![
            Slug::parse("leaf-a").unwrap(),
            Slug::parse("leaf-b").unwrap(),
            Slug::parse("leaf-c").unwrap(),
        ],
    }];
    manifest::write(&manifest_path, &m).unwrap();
    fs::create_dir_all(dir.path().join("branches")).unwrap();
    fs::write(dir.path().join("branches/concept.md"), "old").unwrap();

    // Delete leaf-c to make the branch stale
    fs::remove_file(dir.path().join("leaf-c.md")).unwrap();

    let cfg = make_test_config(dir.path());
    let provider = StaticProvider::new(serde_json::json!({
        "updated_branches": [{
            "slug": "concept",
            "title": "Concept",
            "body": "# Concept\n\nRebuilt from two leaves.",
            "leaves": ["leaf-a", "leaf-b"]
        }],
        "new_branches": []
    }));

    let result = run_compile_with_provider(
        &cfg,
        CompileOptions::default(),
        &provider,
        &cfg.effective_compile_model(),
    )
    .unwrap();

    // Stale repair happens deterministically in pre-pass; no LLM call needed
    assert_eq!(result.status, "noop");
    assert_eq!(provider.calls(), 0);

    let m = manifest::read(&manifest_path).unwrap();
    assert!(m.leaf_by_slug_str("leaf-c").is_none());
    let branch = m.branch_by_slug_str("concept").unwrap();
    assert_eq!(
        branch.leaves.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        vec!["leaf-a", "leaf-b"]
    );
    assert!(!branch.stale);
    assert!(fs::read_to_string(dir.path().join("leaf-a.md"))
        .unwrap()
        .contains("Body for leaf-a"));
}

#[test]
fn stale_branch_below_threshold_removed() {
    let dir = TempDir::new().unwrap();
    seed_manifest(
        dir.path(),
        &[
            ("leaf-a", "A", "https://example.com/a"),
            ("leaf-b", "B", "https://example.com/b"),
        ],
    );
    write_leaf(dir.path(), "leaf-a", "A", "https://example.com/a");
    // leaf-b deliberately missing
    let manifest_path = dir.path().join(".bo/manifest.json");
    let mut m = manifest::read(&manifest_path).unwrap();
    m.tree.last_compiled_at = Some(Timestamp::parse("2026-06-01T12:00:00Z").unwrap());
    m.branches = vec![BranchRecord {
        slug: Slug::parse("doomed").unwrap(),
        file: "branches/doomed.md".to_string(),
        title: Title::new("Doomed"),
        created_at: Timestamp::parse("2026-06-01T12:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-06-01T12:00:00Z").unwrap(),
        stale: false,
        leaves: vec![
            Slug::parse("leaf-a").unwrap(),
            Slug::parse("leaf-b").unwrap(),
        ],
    }];
    manifest::write(&manifest_path, &m).unwrap();
    fs::create_dir_all(dir.path().join("branches")).unwrap();
    fs::write(dir.path().join("branches/doomed.md"), "old").unwrap();

    let cfg = make_test_config(dir.path());
    let provider = StaticProvider::new(empty_incremental_response());

    let result = run_compile_with_provider(
        &cfg,
        CompileOptions::default(),
        &provider,
        &cfg.effective_compile_model(),
    )
    .unwrap();

    // Stale repair happens deterministically; branch below 2-leaf threshold removed
    assert_eq!(result.status, "noop");
    let m = manifest::read(&manifest_path).unwrap();
    assert!(m.branch_by_slug_str("doomed").is_none());
    assert!(!dir.path().join("branches/doomed.md").exists());
    assert!(m.leaf_by_slug_str("leaf-b").is_none());
}
