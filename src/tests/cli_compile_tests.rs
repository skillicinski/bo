use super::{
    compile_error_payload, degenerate_result_warning, plan, render_human, repair, CompileOptions,
    CompileResult, CompileRunMode,
};
use crate::cli::json;
use crate::domain::manifest::{Manifest, TreeMeta};
use crate::domain::{Branch, Leaf, Title, Url};
use crate::domain::{Slug, Timestamp};
use crate::engine::config::SeededConfig;
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

fn seeded_config(dir: &Path) -> SeededConfig {
    let tree_cfg = crate::domain::tree::TreeConfig {
        path: dir.to_path_buf(),
        name: "test-tree".to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
    };
    SeededConfig::new(crate::engine::config::Config::default(), tree_cfg)
}

fn write_manifest(dir: &Path, manifest: &Manifest) {
    let tree = crate::domain::tree::Tree::from_config(&crate::domain::tree::TreeConfig {
        path: dir.to_path_buf(),
        name: "test-tree".to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
    });
    crate::engine::manifest::write(&crate::domain::tree::manifest_path(&tree.path), manifest)
        .unwrap();
}

fn read_manifest(dir: &Path) -> Manifest {
    let manifest_path = crate::domain::tree::manifest_path(dir);
    crate::engine::manifest::read(&manifest_path).unwrap()
}

fn leaf_record(slug: &str, file: &str, title: &str, collected_at: &str) -> Leaf {
    Leaf {
        slug: Slug::generate(slug, ""),
        file: file.to_string(),
        title: Title::parse(title).ok(),
        url: Url::parse("https://example.com").unwrap(),
        collected_at: Timestamp::parse(collected_at).unwrap(),
        summary: Some("summary text".to_string()),
    }
}

fn fresh_manifest(name: &str, created_at: &str, last_compiled_at: Option<&str>) -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: name.to_string(),
            created_at: Timestamp::parse(created_at).unwrap(),
            last_compiled_at: last_compiled_at.map(|s| Timestamp::parse(s).unwrap()),
        },
        leaves: Vec::new(),
        branches: Vec::new(),
    }
}

fn write_leaf(dir: &Path, file: &str, content: &str) {
    let path = dir.join(file);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
}

struct ScriptedAgentProvider {
    responses: Vec<crate::engine::llm::AgentResponse>,
    calls: AtomicUsize,
    messages: Mutex<Vec<Vec<crate::engine::llm::AgentMessage>>>,
    tool_schemas: Mutex<Option<Vec<crate::engine::llm::ToolSchema>>>,
}

impl ScriptedAgentProvider {
    fn new(responses: Vec<crate::engine::llm::AgentResponse>) -> Self {
        Self {
            responses,
            calls: AtomicUsize::new(0),
            messages: Mutex::new(Vec::new()),
            tool_schemas: Mutex::new(None),
        }
    }

    fn messages(&self) -> Vec<Vec<crate::engine::llm::AgentMessage>> {
        self.messages
            .lock()
            .expect("scripted messages poisoned")
            .clone()
    }

    fn tool_schemas(&self) -> Option<Vec<crate::engine::llm::ToolSchema>> {
        self.tool_schemas
            .lock()
            .expect("scripted tool schemas poisoned")
            .clone()
    }
}

#[async_trait]
impl crate::engine::llm::LlmProvider for ScriptedAgentProvider {
    async fn complete(
        &self,
        _: &[crate::engine::llm::Message],
        _: &str,
        _: u32,
        _: Option<&crate::engine::llm::NormalizedSchema>,
        _: bool,
    ) -> Result<crate::engine::llm::LlmResponse, crate::engine::llm::LlmError> {
        unreachable!("agent compile tests only use complete_with_tools")
    }

    async fn complete_with_tools(
        &self,
        messages: &[crate::engine::llm::AgentMessage],
        _: &str,
        _: u32,
        tool_schemas: &[crate::engine::llm::ToolSchema],
        _: bool,
    ) -> Result<crate::engine::llm::AgentResponse, crate::engine::llm::LlmError> {
        self.messages
            .lock()
            .expect("scripted messages poisoned")
            .push(messages.to_vec());
        let mut schemas = self
            .tool_schemas
            .lock()
            .expect("scripted tool schemas poisoned");
        if schemas.is_none() {
            *schemas = Some(tool_schemas.to_vec());
        }
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(self
            .responses
            .get(call)
            .cloned()
            .unwrap_or_else(|| crate::engine::llm::AgentResponse {
                content: Some(String::new()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: crate::engine::llm::FinishReason::Stop,
                usage: None,
            }))
    }
}

fn agent_tool_response(id: &str, name: &str, arguments: &str) -> crate::engine::llm::AgentResponse {
    crate::engine::llm::AgentResponse {
        content: None,
        reasoning_content: None,
        tool_calls: vec![crate::engine::llm::ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
        }],
        finish_reason: crate::engine::llm::FinishReason::Other("tool_calls".to_string()),
        usage: None,
    }
}

fn agent_model() -> crate::engine::llm::Model {
    crate::engine::llm::Model::parse("deepseek-v4-flash", crate::engine::llm::Provider::Deepseek)
        .unwrap()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn missing_unbranched_new_leaf_is_pruned_not_error() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-02-01T00:00:00Z"));
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-03-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-03-01T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody\n");
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 1 orphan"));

    let repaired = read_manifest(dir.path());
    assert_eq!(repaired.leaves.len(), 1);
    assert_eq!(repaired.leaves[0].slug.as_str(), "leaf-b");
}

#[test]
fn missing_unbranched_leaf_never_compiled_is_pruned() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", None);
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-01-15T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody\n");
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 1 orphan"));
    assert_eq!(read_manifest(dir.path()).leaves.len(), 1);
}

#[test]
fn repair_with_no_missing_files_has_empty_notifications() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", None);
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-a.md", "---\ntitle: Leaf A\n---\n\nbody\n");
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

    assert!(notifications.is_empty());
}

#[test]
fn all_leaves_deleted_manifest_repaired_to_empty() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-02-01T00:00:00Z"));
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-03-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-03-01T00:00:00Z",
    ));
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 2 orphan"));
    assert_eq!(read_manifest(dir.path()).leaves.len(), 0);
}

#[test]
fn compile_result_notifications_skipped_from_json() {
    let result = CompileResult {
        status: "compiled".to_string(),
        reason: None,
        mode: Some(super::CompileRunMode::Full),
        model: Some("gpt-4.1".to_string()),
        branches: vec![super::BranchResult {
            slug: "test-branch".to_string(),
            title: "Test Branch".to_string(),
            leaf_count: 2,
        }],
        leaves_processed: 2,
        leaves_skipped: Vec::new(),
        notifications: vec!["pruned 3 orphan leaf records".to_string()],
        warnings: Vec::new(),
    };

    let encoded = json::success_string("compile", &result, Vec::new()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["ok"], true);
    assert!(parsed["data"]["notifications"].is_null());
}

#[test]
fn compile_result_warnings_skipped_from_json() {
    let result = CompileResult {
        status: "compiled".to_string(),
        reason: None,
        mode: Some(super::CompileRunMode::Full),
        model: Some("gpt-4.1".to_string()),
        branches: vec![super::BranchResult {
            slug: "test-branch".to_string(),
            title: "Test Branch".to_string(),
            leaf_count: 2,
        }],
        leaves_processed: 2,
        leaves_skipped: Vec::new(),
        notifications: Vec::new(),
        warnings: vec!["warning: title collision — shared".to_string()],
    };

    let encoded = json::success_string("compile", &result, Vec::new()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    // warnings are presentation (stderr), never part of the JSON envelope.
    assert!(parsed["data"]["warnings"].is_null());
}

fn branch_record(slug: &str, title: &str, leaf_slugs: &[&str]) -> Branch {
    Branch {
        slug: Slug::generate(slug, ""),
        file: format!("branch/{}.md", slug),
        title: Title::parse(title).unwrap(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        leaves: leaf_slugs.iter().map(|s| Slug::generate(s, "")).collect(),
    }
}

#[test]
fn repair_notifications_include_branch_repair_and_removal_messages() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));

    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-c",
        "leaf-c.md",
        "Leaf C",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-d",
        "leaf-d.md",
        "Leaf D",
        "2026-02-01T00:00:00Z",
    ));

    manifest
        .branches
        .push(branch_record("branch-1", "Branch 1", &["leaf-a", "leaf-b"]));
    manifest.branches.push(branch_record(
        "branch-2",
        "Branch 2",
        &["leaf-a", "leaf-b", "leaf-c"],
    ));
    manifest
        .branches
        .push(branch_record("branch-3", "Branch 3", &["leaf-c", "leaf-d"]));

    // Write only leaf-a and leaf-b; leaf-c and leaf-d are missing.
    write_leaf(
        dir.path(),
        "leaf-a.md",
        "---\ntitle: Leaf A\n---\n\nbody a\n",
    );
    write_leaf(
        dir.path(),
        "leaf-b.md",
        "---\ntitle: Leaf B\n---\n\nbody b\n",
    );
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

    // Messages should include branch repair and removal, not just prune.
    let notification_set: HashSet<&str> = notifications.iter().map(String::as_str).collect();
    assert!(
        notification_set
            .iter()
            .any(|n| n.contains("repaired 1 branch")),
        "expected 'repaired 1 branch' in notifications: {:?}",
        notifications
    );
    assert!(
        notification_set
            .iter()
            .any(|n| n.contains("removed 1 stale branch")),
        "expected 'removed 1 stale branch' in notifications: {:?}",
        notifications
    );

    // branch-1 (all leaves present): no repair needed, stays in manifest
    // branch-2 (leaf-c missing): repaired, stays at 2 leaves
    // branch-3 (both leaves missing): removed
    let repaired = read_manifest(dir.path());
    assert_eq!(repaired.branches.len(), 2);
    let branch_slugs: HashSet<&str> = repaired.branches.iter().map(|b| b.slug.as_str()).collect();
    assert!(branch_slugs.contains("branch-1"));
    assert!(branch_slugs.contains("branch-2"));
    assert!(!branch_slugs.contains("branch-3"));
}

#[test]
fn human_output_includes_notifications() {
    let result = CompileResult {
        status: "noop".to_string(),
        reason: Some("empty_tree".to_string()),
        mode: None,
        model: None,
        branches: Vec::new(),
        leaves_processed: 0,
        leaves_skipped: Vec::new(),
        notifications: vec![
            "pruned 1 orphan leaf record (file missing, not in any branch)".to_string(),
        ],
        warnings: Vec::new(),
    };
    let mut stdout = Vec::new();
    render_human(&result, &mut stdout, "test-tree").unwrap();
    let output = String::from_utf8(stdout).unwrap();

    assert!(output.contains("test-tree is empty"));
    assert!(output.contains("\u{2192} pruned 1 orphan"));
}

// ── run-mode selection ─────────────────────────────────────────────────────

#[test]
fn select_run_mode_forces_full_when_no_branches_exist() {
    // A fresh tree (no branches) has nothing to incrementally update, so it
    // must compile full even without --all. Incremental mode against an empty
    // branch graph sends a prompt with no branch context but an incremental
    // response schema, so the LLM cannot produce valid updated_branches.
    let manifest = fresh_manifest("t", "2026-01-01T00:00:00Z", None);
    assert_eq!(
        plan::select_run_mode(
            CompileOptions {
                all: false,
                ..Default::default()
            },
            &manifest
        ),
        CompileRunMode::Full,
        "fresh tree with no branches must compile full even without --all"
    );
}

#[test]
fn select_run_mode_incremental_only_with_branches_and_no_all() {
    use crate::domain::Branch;
    use crate::domain::Title;

    let mut manifest = fresh_manifest("t", "2026-01-01T00:00:00Z", Some("2026-01-02T00:00:00Z"));
    manifest.branches.push(Branch {
        slug: Slug::generate("existing", ""),
        file: "branch/existing.md".to_string(),
        title: Title::parse("existing").unwrap(),
        created_at: Timestamp::parse("2026-01-02T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-01-02T00:00:00Z").unwrap(),
        leaves: vec![Slug::generate("a", "")],
    });

    assert_eq!(
        plan::select_run_mode(
            CompileOptions {
                all: false,
                ..Default::default()
            },
            &manifest
        ),
        CompileRunMode::Incremental,
        "tree with branches and no --all runs incremental"
    );
    assert_eq!(
        plan::select_run_mode(
            CompileOptions {
                all: true,
                ..Default::default()
            },
            &manifest
        ),
        CompileRunMode::Full,
        "--all always forces full"
    );
}

// ── context-mode selection ─────────────────────────────────────────────────

#[test]
fn ensure_compile_context_fits_errors_on_overflow() {
    use crate::cli::compile::execute::ensure_compile_context_fits;
    use crate::engine::llm::{Model, Provider};

    let model = Model::parse("gpt-4.1-mini", Provider::OpenAI).unwrap();

    let small = execute_prompt_tokens(64);
    assert!(ensure_compile_context_fits(&model, small).is_ok());

    let huge = execute_prompt_tokens(usize::MAX);
    assert!(
        ensure_compile_context_fits(&model, huge).is_err(),
        "overflow must error"
    );
}

/// Wrap a byte count into a token estimate comparable to what the compile
/// pipeline computes, so tests exercise the same fit-check path.
fn execute_prompt_tokens(prompt_bytes: usize) -> usize {
    crate::cli::compile::execute::estimate_compile_prompt_tokens(prompt_bytes)
}

#[test]
fn derived_compile_schema_requires_branches() {
    let schema =
        serde_json::to_value(crate::engine::schema::inline_schema_for::<CompileResponse>())
            .unwrap();
    let obj = schema.as_object().expect("top-level is object");
    assert_eq!(obj["additionalProperties"], false);
    let required: Vec<&str> = obj["required"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(required.contains(&"branches"));
}

#[test]
fn derived_incremental_compile_schema_requires_updated_and_new_branches() {
    let schema = serde_json::to_value(crate::engine::schema::inline_schema_for::<
        IncrementalCompileResponse,
    >())
    .unwrap();
    let obj = schema.as_object().expect("top-level is object");
    assert_eq!(obj["additionalProperties"], false);
    let required: Vec<&str> = obj["required"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(required.contains(&"updated_branches"));
    assert!(required.contains(&"new_branches"));
}

// ── leaf reference fidelity ───────────────────────────────────────────────

use super::parse::{
    parse_and_validate_with_input_size, CompileResponse, IncrementalCompileResponse,
};
use super::plan::LoadedLeaf;
use super::validation::leaf_resolver;
use super::CompileError;

fn loaded_leaf(slug: &str, title: &str) -> LoadedLeaf {
    LoadedLeaf {
        slug: slug.to_string(),
        filename: format!("{}.md", slug),
        title: title.to_string(),
        summary: None,
        body: format!("body of {}", title),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

/// Minimal valid full-compile response: one branch over the given leaf refs.
fn branch_response(leaves: &[&str]) -> String {
    let leaves_json = leaves
        .iter()
        .map(|l| format!("\"{}\"", l))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"branches":[{{"title":"Concept","body":"body text","leaves":[{}]}}]}}"#,
        leaves_json
    )
}

#[test]
fn valid_leaf_reference_map_resolves_by_filename_stem_and_unique_title() {
    let leaves = vec![
        loaded_leaf("alpha-concept", "Alpha Concept"),
        loaded_leaf("beta-thing", "Beta Thing"),
    ];
    let lookup = leaf_resolver(&leaves);

    assert!(lookup.collisions.is_empty());
    // filename, stem (= slug), lowercased title, slugified title all resolve.
    assert_eq!(
        lookup.map.get("alpha-concept.md"),
        Some(&"alpha-concept.md".to_string())
    );
    assert_eq!(
        lookup.map.get("alpha-concept"),
        Some(&"alpha-concept.md".to_string())
    );
    assert_eq!(
        lookup.map.get("alpha concept"),
        Some(&"alpha-concept.md".to_string())
    );
    assert_eq!(
        lookup.map.get("beta-thing.md"),
        Some(&"beta-thing.md".to_string())
    );
}

#[test]
fn valid_leaf_reference_map_drops_ambiguous_title_keys() {
    // Two leaves share a title → the title key is removed so a title reference
    // fails validation rather than silently resolving to the wrong leaf.
    let leaves = vec![
        loaded_leaf("gamma-one", "Shared Topic"),
        loaded_leaf("gamma-two", "Shared Topic"),
    ];
    let lookup = leaf_resolver(&leaves);

    assert!(
        !lookup.collisions.is_empty(),
        "expected a collision warning"
    );
    assert!(
        !lookup.map.contains_key("shared topic"),
        "ambiguous title key must be absent"
    );
    // Slugs (stems) stay unique and resolvable.
    assert_eq!(
        lookup.map.get("gamma-one"),
        Some(&"gamma-one.md".to_string())
    );
    assert_eq!(
        lookup.map.get("gamma-two"),
        Some(&"gamma-two.md".to_string())
    );
}

#[test]
fn collision_warnings_captured_as_data_not_printed() {
    // Two leaves share a title → the collision is recorded as a warning string
    // (previously eprintln'd inside validate). Validation still succeeds when a
    // branch references the leaves by their unique slug/filename.
    let leaves = vec![
        loaded_leaf("gamma-one", "Shared Topic"),
        loaded_leaf("gamma-two", "Shared Topic"),
    ];
    let mut warnings = Vec::new();
    parse_and_validate_with_input_size(
        &branch_response(&["gamma-one", "gamma-two.md"]),
        &leaves,
        1024,
        &mut warnings,
    )
    .expect("refs by unique slug/filename must resolve despite title collision");

    assert!(
        warnings
            .iter()
            .any(|w| w.contains("warning: title collision") && w.contains("Shared Topic")),
        "expected a title-collision warning captured as data: {:?}",
        warnings
    );
}

#[test]
fn parse_resolves_leaf_references_by_slug_filename_and_title() {
    let leaves = vec![
        loaded_leaf("alpha-concept", "Alpha Concept"),
        loaded_leaf("beta-thing", "Beta Thing"),
    ];

    // slug/stem + filename both resolve and normalize to the canonical filename.
    let plan = parse_and_validate_with_input_size(
        &branch_response(&["alpha-concept", "beta-thing.md"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .expect("refs by slug and filename must resolve");
    assert_eq!(
        plan.branches[0].leaves,
        vec!["alpha-concept.md", "beta-thing.md"]
    );

    // unique title resolves (case-insensitive).
    parse_and_validate_with_input_size(
        &branch_response(&["Alpha Concept", "Beta Thing"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .expect("refs by unique title must resolve");
}

#[test]
fn parse_rejects_invented_leaf_reference() {
    let leaves = vec![
        loaded_leaf("alpha-concept", "Alpha Concept"),
        loaded_leaf("beta-thing", "Beta Thing"),
    ];
    let err = parse_and_validate_with_input_size(
        &branch_response(&["alpha-concept", "invented-name"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, CompileError::Validation(ref msg) if msg.contains("unknown leaf")),
        "invented leaf reference must be a validation error: {:?}",
        err
    );
}

#[test]
fn parse_rejects_ambiguous_title_reference() {
    let leaves = vec![
        loaded_leaf("gamma-one", "Shared Topic"),
        loaded_leaf("gamma-two", "Shared Topic"),
        loaded_leaf("alpha-concept", "Alpha Concept"),
    ];
    let err = parse_and_validate_with_input_size(
        &branch_response(&["Shared Topic", "alpha-concept"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, CompileError::Validation(ref msg) if msg.contains("unknown leaf")),
        "ambiguous title reference must fail validation, not silently resolve: {:?}",
        err
    );
}

// ── leaf multi-membership ────────────────────────────────────────────────────

#[test]
fn derived_compile_schema_has_no_ref_or_defs_or_schema_key() {
    let schema =
        serde_json::to_value(crate::engine::schema::inline_schema_for::<CompileResponse>())
            .unwrap();
    let json_str = schema.to_string();
    assert!(!json_str.contains("\"$schema\""));
    assert!(!json_str.contains("\"definitions\""));
    assert!(!json_str.contains("\"$ref\""));
}

#[test]
fn derived_incremental_compile_schema_has_no_ref_or_defs_or_schema_key() {
    let schema = serde_json::to_value(crate::engine::schema::inline_schema_for::<
        IncrementalCompileResponse,
    >())
    .unwrap();
    let json_str = schema.to_string();
    assert!(!json_str.contains("\"$schema\""));
    assert!(!json_str.contains("\"definitions\""));
    assert!(!json_str.contains("\"$ref\""));
}

#[test]
fn build_manifest_delta_allows_one_leaf_in_multiple_branches() {
    use super::plan::build_manifest_delta;
    use super::validation::{CompilePlan, ValidatedBranch};
    use super::CompileRunMode;

    // One leaf participates in two cross-cutting concepts. The manifest model
    // stores branch→leaf as independent lists, so the same slug may appear in
    // several branches; the inverse is computed by branches_for_leaf.
    let plan = CompilePlan {
        branches: vec![
            ValidatedBranch {
                slug: "concept-a".to_string(),
                title: "Concept A".to_string(),
                body: "body a".to_string(),
                leaves: vec!["shared-leaf.md".to_string(), "alpha.md".to_string()],
            },
            ValidatedBranch {
                slug: "concept-b".to_string(),
                title: "Concept B".to_string(),
                body: "body b".to_string(),
                leaves: vec!["shared-leaf.md".to_string(), "beta.md".to_string()],
            },
        ],
    };

    let current = fresh_manifest("t", "2026-01-01T00:00:00Z", None);
    let ts = Timestamp::parse("2026-06-28T00:00:00Z").unwrap();
    let delta = build_manifest_delta(&current, &plan, CompileRunMode::Full, &ts).unwrap();

    assert_eq!(delta.new_manifest.branches.len(), 2);
    assert_eq!(
        delta.branches_created.len(),
        2,
        "both branches are new in Full mode"
    );

    let shared_slug = Slug::parse("shared-leaf").unwrap();
    let containing: Vec<&str> = delta
        .new_manifest
        .branches_for_leaf(&shared_slug)
        .iter()
        .map(|b| b.slug.as_str())
        .collect();
    assert_eq!(
        containing,
        vec!["concept-a", "concept-b"],
        "a leaf must be allowed in multiple branches"
    );
}

// ── full-mode parse validation gaps ──────────────────────────────────────

#[test]
fn parse_full_rejects_empty_title() {
    let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
    let json = r#"{"branches":[{"title":"","body":"some body","leaves":["a","b"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("empty title")));
}

#[test]
fn parse_full_rejects_empty_body() {
    let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
    let json = r#"{"branches":[{"title":"Concept","body":"","leaves":["a","b"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("empty body")));
}

#[test]
fn parse_full_rejects_duplicate_slug() {
    let leaves = vec![
        loaded_leaf("a", "A"),
        loaded_leaf("b", "B"),
        loaded_leaf("c", "C"),
        loaded_leaf("d", "D"),
    ];
    // Two branches with the same title → same slug → duplicate slug error
    let json = r#"{"branches":[{"title":"Same Thing","body":"body","leaves":["a","b"]},{"title":"Same Thing","body":"body","leaves":["c","d"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(
        matches!(err, CompileError::Validation(ref msg) if msg.contains("duplicate branch slug"))
    );
}

#[test]
fn parse_full_rejects_single_leaf_branch() {
    let leaves = vec![
        loaded_leaf("a", "A"),
        loaded_leaf("b", "B"),
        loaded_leaf("c", "C"),
    ];
    let json = r#"{"branches":[{"title":"Concept","body":"body","leaves":["a"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("at least 2 leaves")));
}

// ── incremental-mode parse validation ─────────────────────────────────────

use super::parse::parse_and_validate_incremental_with_input_size;

/// Minimal valid incremental response helper.
fn incremental_response(updated: &str, new: &str) -> String {
    format!(
        r#"{{"updated_branches":{},"new_branches":{}}}"#,
        updated, new
    )
}

/// Set up a tree on disk with 4 leaves, 1 existing branch (covering the 2 older
/// leaves), and fresh leaf files. Returns config, manifest, and loaded leaves.
fn setup_incremental_tree(dir: &Path) -> (SeededConfig, Manifest, Vec<LoadedLeaf>) {
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));
    // leaf-a, leaf-b: new (collected after last_compile)
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-01-15T00:00:00Z",
    ));
    // leaf-c, leaf-d: existing (collected before last_compile, already branched)
    manifest.leaves.push(leaf_record(
        "leaf-c",
        "leaf-c.md",
        "Leaf C",
        "2026-01-05T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-d",
        "leaf-d.md",
        "Leaf D",
        "2026-01-05T00:00:00Z",
    ));
    manifest.branches.push(branch_record(
        "existing",
        "Existing Branch",
        &["leaf-c", "leaf-d"],
    ));

    write_leaf(dir, "leaf-a.md", "---\ntitle: Leaf A\n---\n\nbody a\n");
    write_leaf(dir, "leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody b\n");
    write_leaf(dir, "leaf-c.md", "---\ntitle: Leaf C\n---\n\nbody c\n");
    write_leaf(dir, "leaf-d.md", "---\ntitle: Leaf D\n---\n\nbody d\n");

    std::fs::create_dir_all(dir.join("branch")).unwrap();
    std::fs::write(
        dir.join("branch/existing.md"),
        "---\ntitle: Existing Branch\n---\n\n# Existing Branch\n\nbody\n",
    )
    .unwrap();

    write_manifest(dir, &manifest);

    let cfg = seeded_config(dir);
    let loaded = vec![
        loaded_leaf("leaf-a", "Leaf A"),
        loaded_leaf("leaf-b", "Leaf B"),
        loaded_leaf("leaf-c", "Leaf C"),
        loaded_leaf("leaf-d", "Leaf D"),
    ];
    (cfg, manifest, loaded)
}

fn incremental_update_submission(slug: &str, leaves: &[&str]) -> String {
    serde_json::json!({
        "updated_branches": [{
            "slug": slug,
            "title": "Existing Branch",
            "body": "updated body",
            "leaves": leaves,
        }],
        "new_branches": [],
    })
    .to_string()
}

#[test]
fn agent_incremental_branch_identifier_round_trips_from_list_branches() {
    let dir = TempDir::new().unwrap();
    let (cfg, manifest, loaded) = setup_incremental_tree(dir.path());
    let submission =
        incremental_update_submission("branch/existing", &["leaf-c", "leaf-d", "leaf-a"]);
    let provider = ScriptedAgentProvider::new(vec![
        agent_tool_response("list_branches", "list_branches", "{}"),
        agent_tool_response("submit", "submit_compile", &submission),
    ]);
    let model = agent_model();

    let (plan, stats, _) = super::agent::run_agent_dry_run(
        &cfg,
        &provider,
        &model,
        &manifest,
        &loaded,
        CompileRunMode::Incremental,
    )
    .expect("branch/<slug> submission should validate");

    assert_eq!(plan.branches[0].slug, "existing");
    assert_eq!(stats.turns, 2);
    let messages = provider.messages();
    let list_result = messages[1]
        .iter()
        .find_map(|message| match message {
            crate::engine::llm::AgentMessage::Tool(result)
                if result.tool_call_id == "list_branches" =>
            {
                Some(result.content.as_str())
            }
            _ => None,
        })
        .expect("second turn should receive list_branches output");
    let listed: serde_json::Value = serde_json::from_str(list_result).unwrap();
    assert_eq!(listed["branches"][0]["slug"], "branch/existing");
}

#[test]
fn agent_incremental_bare_branch_slug_still_validates() {
    let dir = TempDir::new().unwrap();
    let (cfg, manifest, loaded) = setup_incremental_tree(dir.path());
    let submission = incremental_update_submission("existing", &["leaf-c", "leaf-d", "leaf-a"]);
    let provider = ScriptedAgentProvider::new(vec![agent_tool_response(
        "submit",
        "submit_compile",
        &submission,
    )]);
    let model = agent_model();

    let (plan, _, _) = super::agent::run_agent_dry_run(
        &cfg,
        &provider,
        &model,
        &manifest,
        &loaded,
        CompileRunMode::Incremental,
    )
    .expect("bare branch slug submission should validate");

    assert_eq!(plan.branches[0].slug, "existing");
}

#[test]
fn one_shot_incremental_rejects_branch_identifier_prefix() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());
    let submission =
        incremental_update_submission("branch/existing", &["leaf-c", "leaf-d", "leaf-a"]);

    let err = parse_and_validate_incremental_with_input_size(
        &submission,
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .expect_err("one-shot incremental validation must retain bare branch slugs");

    assert!(
        matches!(err, CompileError::Validation(ref message) if message.contains("unknown branch 'branch/existing'")),
        "expected bare-slug validation failure, got: {err:?}"
    );
}

#[test]
fn agent_rejects_branch_identifiers_in_leaf_lists_with_teaching_hint() {
    for identifier in ["branch/existing", "existing"] {
        let dir = TempDir::new().unwrap();
        let (cfg, manifest, loaded) = setup_incremental_tree(dir.path());
        let submission =
            incremental_update_submission("existing", &["leaf-c", "leaf-d", identifier]);
        let provider = ScriptedAgentProvider::new(vec![agent_tool_response(
            "submit",
            "submit_compile",
            &submission,
        )]);
        let model = agent_model();

        let err = super::agent::run_agent_dry_run(
            &cfg,
            &provider,
            &model,
            &manifest,
            &loaded,
            CompileRunMode::Incremental,
        )
        .expect_err("branch identifiers must not resolve as leaves");

        match err {
            CompileError::AgentFailed {
                last_error: Some(message),
                ..
            } => {
                assert!(
                    message.contains(&format!("unknown leaf '{identifier}'")),
                    "expected unknown-leaf error, got: {message}"
                );
                assert!(
                    message.contains(&format!(
                        ": {identifier} is a branch slug, not a leaf; leaf lists may only contain leaf slugs (see list_leaves)"
                    )),
                    "expected teaching hint, got: {message}"
                );
            }
            other => panic!("expected AgentFailed with teaching hint, got: {other:?}"),
        }
    }
}

#[test]
fn parse_incremental_update_preserves_existing_leaves_and_adds_new() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    // Update existing branch: preserves leaf-c, leaf-d and adds leaf-a
    let updated = r#"[{"slug":"existing","title":"Existing Branch","body":"updated body","leaves":["leaf-c","leaf-d","leaf-a"]}]"#;
    let new = r#"[]"#;
    let plan = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap();

    assert_eq!(plan.branches.len(), 1);
    assert_eq!(plan.branches[0].slug, "existing");
    assert!(plan.branches[0].leaves.contains(&"leaf-a.md".to_string()));
    assert!(plan.branches[0].leaves.contains(&"leaf-c.md".to_string()));
    assert!(plan.branches[0].leaves.contains(&"leaf-d.md".to_string()));
}

#[test]
fn parse_incremental_update_dropping_existing_leaf_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    // Update that omits leaf-d (an existing leaf) — not allowed
    let updated = r#"[{"slug":"existing","title":"Existing Branch","body":"body","leaves":["leaf-c","leaf-a"]}]"#;
    let new = r#"[]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, CompileError::Validation(ref msg) if msg.contains("dropped existing leaf"))
    );
}

#[test]
fn parse_incremental_new_branch_without_new_leaf_is_dropped() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    // New branch references only old leaves (no new leaf integrated)
    let updated = r#"[]"#;
    let new = r#"[{"title":"Reorganised","body":"body","leaves":["leaf-c","leaf-d"]}]"#;
    let plan = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap();

    // Branch is silently dropped (no new leaves)
    assert!(
        plan.branches.is_empty(),
        "new branch without new leaf must be dropped"
    );
}

#[test]
fn parse_incremental_update_unknown_branch_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    let updated =
        r#"[{"slug":"nonexistent","title":"Whatever","body":"body","leaves":["leaf-a","leaf-b"]}]"#;
    let new = r#"[]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("unknown branch")));
}

#[test]
fn parse_incremental_title_change_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    let updated = r#"[{"slug":"existing","title":"Different Title","body":"body","leaves":["leaf-c","leaf-d","leaf-a"]}]"#;
    let new = r#"[]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("changed title")));
}

#[test]
fn parse_incremental_new_branch_empty_title_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    let updated = r#"[]"#;
    let new = r#"[{"title":"","body":"body","leaves":["leaf-a","leaf-b"]}]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("empty title")));
}

#[test]
fn parse_incremental_new_branch_empty_body_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    let updated = r#"[]"#;
    let new = r#"[{"title":"Valid Title","body":"","leaves":["leaf-a","leaf-b"]}]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("empty body")));
}

#[test]
fn parse_incremental_update_with_no_new_leaf_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    // Update that adds no new leaf
    let updated = r#"[{"slug":"existing","title":"Existing Branch","body":"body","leaves":["leaf-c","leaf-d"]}]"#;
    let new = r#"[]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, CompileError::Validation(ref msg) if msg.contains("no newly processed leaf"))
    );
}

#[test]
fn parse_incremental_insufficient_leaves_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    // New branch with only 1 leaf
    let updated = r#"[]"#;
    let new = r#"[{"title":"Solo","body":"body","leaves":["leaf-a"]}]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("at least 2 leaves")));
}

// ── repair: branch frontmatter consistency ───────────────────────────────

#[test]
fn repair_stale_branches_fixes_branch_frontmatter() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));

    // 3 leaves, all new (collected after last_compile)
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-c",
        "leaf-c.md",
        "Leaf C",
        "2026-02-01T00:00:00Z",
    ));

    // One branch covering all 3 leaves
    manifest.branches.push(branch_record(
        "test-branch",
        "Test Branch",
        &["leaf-a", "leaf-b", "leaf-c"],
    ));

    // Write leaf files (leaf-a intentionally absent/deleted)
    write_leaf(
        dir.path(),
        "leaf-b.md",
        "---\ntitle: Leaf B\n---\n\nbody b\n",
    );
    write_leaf(
        dir.path(),
        "leaf-c.md",
        "---\ntitle: Leaf C\n---\n\nbody c\n",
    );

    // Write branch file with 3 leaves in frontmatter
    std::fs::create_dir_all(dir.path().join("branch")).unwrap();
    let branch_content = "---\ntitle: Test Branch\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\nleaves:\n- leaf-a.md\n- leaf-b.md\n- leaf-c.md\n---\n\n# Test Branch\n\nBody text with reference to Leaf A\n";
    std::fs::write(dir.path().join("branch/test-branch.md"), branch_content).unwrap();

    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

    // Notification should mention frontmatter repair
    assert!(
        notifications
            .iter()
            .any(|n| n.contains("frontmatter repaired")),
        "expected frontmatter repair notification in: {:?}",
        notifications
    );

    // Branch file frontmatter leaves: should have 2 entries (leaf-b, leaf-c),
    // not 3.
    let repaired = std::fs::read_to_string(dir.path().join("branch/test-branch.md")).unwrap();
    assert!(repaired.contains("- leaf-b.md"));
    assert!(repaired.contains("- leaf-c.md"));
    assert!(!repaired.contains("- leaf-a.md"));

    // Body is preserved (stale reference to leaf-a stays)
    assert!(repaired.contains("reference to Leaf A"));

    // Manifest branch leaves should match: 2 entries
    let repaired_manifest = read_manifest(dir.path());
    let branch = repaired_manifest
        .branches
        .iter()
        .find(|b| b.slug.as_str() == "test-branch")
        .unwrap();
    assert_eq!(branch.leaves.len(), 2);
}

// ── degenerate result warning ────────────────────────────────────────────

use super::BranchResult;

fn branch_result(slug: &str, leaf_count: usize) -> BranchResult {
    BranchResult {
        slug: slug.to_string(),
        title: slug.to_string(),
        leaf_count,
    }
}

#[test]
fn degenerate_warning_when_single_branch_for_many_leaves() {
    // gpt-4.1 at 64 leaves silently produced 1 branch / 2 leaves.
    // <2 branches for >20 leaves is degenerate.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[branch_result("catch-all", 2)],
        64,
    );
    let msg = warning.expect("expected a degenerate warning");
    assert!(msg.contains("degenerate compile result"));
    assert!(msg.contains("1 branch"));
    assert!(msg.contains("64 leaves"));
}

#[test]
fn degenerate_warning_when_most_leaves_unbranched() {
    // 3 branches covering only 5 of 30 leaves = 83% unbranched.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[
            branch_result("a", 2),
            branch_result("b", 2),
            branch_result("c", 1),
        ],
        30,
    );
    let msg = warning.expect("expected a degenerate warning");
    assert!(msg.contains("degenerate compile result"));
    assert!(msg.contains("25 of 30 leaves unbranched"));
}

#[test]
fn no_degenerate_warning_for_normal_full_compile() {
    // 3 branches covering 28 of 30 leaves = 7% unbranched, well within bounds.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[
            branch_result("a", 10),
            branch_result("b", 10),
            branch_result("c", 8),
        ],
        30,
    );
    assert!(warning.is_none());
}

#[test]
fn no_degenerate_warning_for_small_corpus() {
    // 20 leaves or fewer are never warned about, even with 0 branches.
    let warning = degenerate_result_warning(Some(CompileRunMode::Full), &[], 20);
    assert!(warning.is_none());
}

#[test]
fn no_degenerate_warning_for_incremental_mode() {
    // Incremental mode never produces degenerate warnings — it naturally
    // produces fewer branches by design.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Incremental),
        &[branch_result("single", 2)],
        64,
    );
    assert!(warning.is_none());
}

#[test]
fn degenerate_warning_low_coverage_ratio() {
    // 2 branches, 66 leaves processed, but branches only claim 15 leaves
    // total (7+8). Coverage = 15/66 ≈ 0.23, below the 0.30 threshold.
    // The unbranched heuristic (77% < 80%) does NOT fire, so this
    // exercises the new coverage-ratio path exclusively.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[branch_result("concept-a", 7), branch_result("concept-b", 8)],
        66,
    );
    let msg = warning.expect("expected a degenerate warning from low coverage ratio");
    assert!(msg.contains("degenerate compile result"));
    assert!(msg.contains("only 15 of 66 leaves placed in branches"));
}

#[test]
fn no_degenerate_warning_for_healthy_coverage() {
    // 3 branches covering 26 of 30 leaves = 87% coverage, 13% unbranched.
    // Both the unbranched (>80%) and coverage (<30%) guards pass through.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[
            branch_result("a", 10),
            branch_result("b", 9),
            branch_result("c", 7),
        ],
        30,
    );
    assert!(warning.is_none());
}

#[test]
fn degenerate_warning_single_branch_regression() {
    // Regression guard: branch_count < 2 with >20 leaves still warns.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[branch_result("catch-all", 2)],
        64,
    );
    assert!(warning.is_some());
}

#[test]
fn degenerate_warning_unbranched_regression() {
    // Regression guard: >80% unbranched still warns.
    // 3 branches, 5 of 30 leaves branched → 25 unbranched (83%).
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[
            branch_result("a", 2),
            branch_result("b", 2),
            branch_result("c", 1),
        ],
        30,
    );
    assert!(warning.is_some());
}

#[test]
fn compile_error_payload_routes_terminal_errors() {
    use std::time::Duration;
    let slugs: &[String] = &[];
    let duration = Duration::from_millis(10);

    // Validation keeps its own shape: validation_failures, no error field.
    let payload = compile_error_payload(
        CompileRunMode::Full,
        slugs,
        &CompileError::Validation("branch #1 has empty title".to_string()),
        duration,
    )
    .expect("validation is journaled");
    assert_eq!(
        payload.validation_failures,
        vec!["branch #1 has empty title".to_string()]
    );
    assert!(payload.error.is_none());

    // LLM/provider failures: error field, empty deltas.
    let llm_errors = [
        CompileError::Truncated,
        CompileError::ContentFilter,
        CompileError::Llm("upstream timeout".to_string()),
        CompileError::ContextOverflow {
            model: "gpt-4.1".to_string(),
            estimated_tokens: Some(200_000),
            context_tokens: Some(128_000),
        },
    ];
    for error in &llm_errors {
        let payload = compile_error_payload(CompileRunMode::Full, slugs, error, duration)
            .expect("LLM/provider error is journaled");
        assert!(payload.validation_failures.is_empty());
        let err = payload.error.expect("error field present");
        assert!(!err.code.is_empty());
        assert!(!err.message.is_empty());
    }

    // Infrastructure / dry-run / agent failures are not compile verdicts.
    for error in [
        CompileError::Io("disk full".to_string()),
        CompileError::Busy("locked".to_string()),
        CompileError::DryRunBlocked("stale".to_string()),
        CompileError::AgentFailed {
            message: "limit".to_string(),
            turns: 0,
            tool_calls: 0,
            usage: None,
            last_error: None,
        },
    ] {
        assert!(
            compile_error_payload(CompileRunMode::Full, slugs, &error, duration).is_none(),
            "{:?} should not be journaled",
            error
        );
    }
}

// ── agent: read_branch + six-tool verification ──────────────────────────────

use super::agent::run_agent_dry_run;

#[test]
fn agent_provides_six_tools_including_read_branch() {
    let cases = [
        (
            CompileRunMode::Incremental,
            incremental_update_submission("branch/existing", &["leaf-c", "leaf-d", "leaf-a"]),
        ),
        (
            CompileRunMode::Full,
            serde_json::json!({
                "branches": [{
                    "title": "Full Branch",
                    "body": "full body",
                    "leaves": ["leaf-c", "leaf-d"],
                }],
            })
            .to_string(),
        ),
    ];

    for (run_mode, submission) in cases {
        let dir = TempDir::new().unwrap();
        let (cfg, manifest, loaded) = setup_incremental_tree(dir.path());
        let provider = ScriptedAgentProvider::new(vec![agent_tool_response(
            "submit",
            "submit_compile",
            &submission,
        )]);
        let model = agent_model();

        run_agent_dry_run(&cfg, &provider, &model, &manifest, &loaded, run_mode)
            .unwrap_or_else(|error| panic!("{run_mode:?} agent run should complete: {error}"));

        let schemas = provider
            .tool_schemas()
            .expect("tool schemas must be captured on first call");
        let names: HashSet<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names.len(), 6, "expected six tools: {names:?}");
        assert_eq!(
            names,
            [
                "list_leaves",
                "list_branches",
                "read_branch",
                "read_leaf",
                "search_corpus",
                "submit_compile",
            ]
            .into_iter()
            .collect(),
        );

        let list_branches_schema = schemas
            .iter()
            .find(|s| s.name == "list_branches")
            .expect("list_branches must be present");
        assert!(
            list_branches_schema.description.contains("branch/<slug>")
                && list_branches_schema.description.contains("read_branch"),
            "list_branches description must explain branch identifiers and read_branch: {}",
            list_branches_schema.description
        );

        let first_messages = &provider.messages()[0];
        let system_msg = first_messages
            .iter()
            .find_map(|m| match m {
                crate::engine::llm::AgentMessage::System(s) => Some(s.as_str()),
                _ => None,
            })
            .expect("system prompt must be present");
        assert!(
            system_msg.contains("Only the six provided tools exist"),
            "system prompt must mention six tools: {}",
            system_msg
        );
        assert!(
            system_msg.contains("list_branches and read_branch"),
            "system prompt must guide list_branches+read_branch: {}",
            system_msg
        );
    }
}

#[test]
fn agent_read_branch_returns_body_leaves_and_handles_pagination() {
    let dir = TempDir::new().unwrap();
    let (cfg, manifest, loaded) = setup_incremental_tree(dir.path());
    let heading = "# Existing Branch\n\n";
    let body = format!("{heading}{}\u{2014}{}", "a".repeat(8190), "z".repeat(12000));
    let em_dash_offset = heading.len() + 8190;
    let branch_content = format!("---\ntitle: Existing Branch\n---\n\n{body}");
    std::fs::write(dir.path().join("branch/existing.md"), &branch_content).unwrap();

    let submission =
        incremental_update_submission("branch/existing", &["leaf-c", "leaf-d", "leaf-a"]);
    let pagination_args = serde_json::json!({
        "slug": "branch/existing",
        "offset": em_dash_offset + 1,
        "limit": 8192,
    })
    .to_string();
    let provider = ScriptedAgentProvider::new(vec![
        agent_tool_response("rb1", "read_branch", r#"{"slug":"branch/existing"}"#),
        agent_tool_response("rb2", "read_branch", r#"{"slug":"existing"}"#),
        agent_tool_response("rb3", "read_branch", &pagination_args),
        agent_tool_response("submit", "submit_compile", &submission),
    ]);
    let model = agent_model();

    let (plan, _, _) = run_agent_dry_run(
        &cfg,
        &provider,
        &model,
        &manifest,
        &loaded,
        CompileRunMode::Incremental,
    )
    .expect("agent run should complete");
    assert_eq!(plan.branches[0].slug, "existing");

    let messages = provider.messages();
    let rb1_result = find_tool_result(&messages, "rb1").expect("rb1 result must be present");
    let rb2_result = find_tool_result(&messages, "rb2").expect("rb2 result must be present");
    for result in [&rb1_result, &rb2_result] {
        assert_eq!(result["slug"], "branch/existing");
        assert_eq!(result["title"], "Existing Branch");
        assert_eq!(result["leaves"], serde_json::json!(["leaf-c", "leaf-d"]));
    }
    assert_eq!(rb1_result["body"], rb2_result["body"]);

    let rb3_result = find_tool_result(&messages, "rb3").expect("rb3 result must be present");
    assert_eq!(rb3_result["offset"].as_u64(), Some(em_dash_offset as u64));
    assert_eq!(rb3_result["total_bytes"].as_u64(), Some(body.len() as u64));
    assert!(
        rb3_result["truncated"].as_bool().unwrap(),
        "truncated must be true for body > 8192 bytes at offset"
    );
    let body_slice = rb3_result["body"].as_str().unwrap();
    assert!(
        body_slice.len() <= 8192 + 2,
        "body slice is bounded, got {} bytes",
        body_slice.len()
    );
}

#[test]
fn agent_read_branch_rejects_unknown_slugs() {
    for identifier in ["nonexistent", "branch/nonexistent", "branch/"] {
        let dir = TempDir::new().unwrap();
        let (cfg, manifest, loaded) = setup_incremental_tree(dir.path());
        let submission = incremental_update_submission("existing", &["leaf-c", "leaf-d", "leaf-a"]);
        let args = serde_json::json!({"slug": identifier}).to_string();
        let provider = ScriptedAgentProvider::new(vec![
            agent_tool_response("rb", "read_branch", &args),
            agent_tool_response("submit", "submit_compile", &submission),
        ]);
        let model = agent_model();

        let (plan, _, _) = run_agent_dry_run(
            &cfg,
            &provider,
            &model,
            &manifest,
            &loaded,
            CompileRunMode::Incremental,
        )
        .expect("agent run should complete");
        assert_eq!(plan.branches[0].slug, "existing");

        let messages = provider.messages();
        let error = find_tool_result(&messages, "rb").expect("rb error result must be present");
        let expected = format!("unknown branch: {identifier}");
        assert_eq!(error["error"].as_str(), Some(expected.as_str()));
    }
}

#[test]
fn agent_read_leaf_rejects_branch_identifiers_with_hint() {
    for identifier in ["existing", "branch/existing"] {
        let dir = TempDir::new().unwrap();
        let (cfg, manifest, loaded) = setup_incremental_tree(dir.path());
        let submission = incremental_update_submission("existing", &["leaf-c", "leaf-d", "leaf-a"]);

        let args = serde_json::json!({"slug": identifier}).to_string();
        let provider = ScriptedAgentProvider::new(vec![
            agent_tool_response("rl", "read_leaf", &args),
            agent_tool_response("submit", "submit_compile", &submission),
        ]);
        let model = agent_model();

        let (plan, _, _) = run_agent_dry_run(
            &cfg,
            &provider,
            &model,
            &manifest,
            &loaded,
            CompileRunMode::Incremental,
        )
        .expect("agent run should complete");
        assert_eq!(plan.branches[0].slug, "existing");

        let messages = provider.messages();
        let error = find_tool_result(&messages, "rl").expect("rl error result must be present");
        let expected = format!("{identifier} is a branch; use read_branch");
        assert_eq!(error["error"].as_str(), Some(expected.as_str()));
    }
}

/// Find the first replayed tool result for a call id.
fn find_tool_result(
    messages: &[Vec<crate::engine::llm::AgentMessage>],
    tool_call_id: &str,
) -> Option<serde_json::Value> {
    let content = messages.iter().find_map(|turn_messages| {
        turn_messages.iter().find_map(|message| match message {
            crate::engine::llm::AgentMessage::Tool(result)
                if result.tool_call_id == tool_call_id =>
            {
                Some(result.content.as_str())
            }
            _ => None,
        })
    })?;
    serde_json::from_str(content).ok()
}

// ── resource envelope ────────────────────────────────────────────────────────

#[test]
fn resource_limit_constants_have_expected_values() {
    assert_eq!(crate::engine::agent::MAX_TURNS, 8);
    assert_eq!(crate::engine::agent::MAX_TOOL_CALLS_PER_RESPONSE, 8);
    assert_eq!(crate::engine::agent::MAX_TOTAL_TOOL_CALLS, 48);
}

// ── cluster tests ───────────────────────────────────────────────────────────

use super::cluster::{
    self, BranchAssignment, ClusterAssignment, ClusterResponse, IncrementalClusterResponse,
};
mod cluster_tests {
    use super::*;

    /// Build a LoadedLeaf with a body that creates similarity with
    /// other leaves sharing the same keyword.
    fn leaf_with_body(slug: &str, title: &str, body: &str) -> LoadedLeaf {
        LoadedLeaf {
            slug: slug.to_string(),
            filename: format!("{}.md", slug),
            title: title.to_string(),
            summary: None,
            body: body.to_string(),
            collected_at: "2026-01-01T00:00:00Z".to_string(),
        }
    }

    // ── deterministic pre-pass ──────────────────────────────────────────

    #[test]
    fn compute_candidate_clusters_groups_similar_leaves() {
        let leaves: Vec<LoadedLeaf> = vec![
            leaf_with_body(
                "rust-1",
                "Rust Ownership",
                "rust borrow checker memory safety ownership",
            ),
            leaf_with_body(
                "rust-2",
                "Rust Traits",
                "rust trait system generics type safety abstraction",
            ),
            leaf_with_body(
                "python-1",
                "Python Decorators",
                "python decorator function wrapper metaprogramming",
            ),
            leaf_with_body(
                "python-2",
                "Python Generators",
                "python generator yield iterator lazy evaluation",
            ),
        ];
        let leaf_refs: Vec<&LoadedLeaf> = leaves.iter().collect();
        let clusters = cluster::compute_candidate_clusters(&leaf_refs);

        // Expect at least one cluster: rust-1 and rust-2 share "rust".
        // python-1 and python-2 share "python".
        assert!(!clusters.is_empty(), "should find at least one cluster");

        // Verify that leaves sharing keywords end up in the same cluster.
        let has_rust_cluster = clusters
            .iter()
            .any(|c| c.leaf_indices.contains(&0) && c.leaf_indices.contains(&1));
        let has_python_cluster = clusters
            .iter()
            .any(|c| c.leaf_indices.contains(&2) && c.leaf_indices.contains(&3));
        assert!(has_rust_cluster, "rust leaves should cluster together");
        assert!(has_python_cluster, "python leaves should cluster together");
    }

    #[test]
    fn compute_candidate_clusters_empty_for_single_leaf() {
        let leaves: Vec<LoadedLeaf> = vec![leaf_with_body("only", "Only", "single leaf body")];
        let leaf_refs: Vec<&LoadedLeaf> = leaves.iter().collect();
        let clusters = cluster::compute_candidate_clusters(&leaf_refs);
        assert!(clusters.is_empty());
    }

    #[test]
    fn compute_candidate_clusters_no_clusters_for_unrelated() {
        let leaves: Vec<LoadedLeaf> = vec![
            leaf_with_body("a", "Art", "painting color canvas brush art"),
            leaf_with_body("b", "Bridge", "concrete steel engineering bridge"),
            leaf_with_body("c", "Cooking", "recipe kitchen food cooking"),
        ];
        let leaf_refs: Vec<&LoadedLeaf> = leaves.iter().collect();
        let clusters = cluster::compute_candidate_clusters(&leaf_refs);
        // Zero shared terms means no clusters.
        assert!(clusters.is_empty());
    }

    // ── cluster validation (Full mode) ───────────────────────────────────

    #[test]
    fn validate_clusters_accepts_valid_response() {
        let leaves = vec![
            loaded_leaf("leaf-1", "Rust"),
            loaded_leaf("leaf-2", "Cargo"),
            loaded_leaf("leaf-3", "Python"),
            loaded_leaf("leaf-4", "Pip"),
        ];
        let response = ClusterResponse {
            clusters: vec![
                ClusterAssignment {
                    title: "Rust Ecosystem".to_string(),
                    leaf_slugs: vec!["leaf-1".to_string(), "leaf-2".to_string()],
                },
                ClusterAssignment {
                    title: "Python Ecosystem".to_string(),
                    leaf_slugs: vec!["leaf-3".to_string(), "leaf-4".to_string()],
                },
            ],
        };
        let mut warnings = Vec::new();
        let result = cluster::validate_clusters(&response, &leaves, &mut warnings);
        assert!(
            result.is_ok(),
            "valid clusters should pass: {:?}",
            result.err()
        );
        let validated = result.unwrap();
        assert_eq!(validated.clusters.len(), 2);
        assert_eq!(validated.clusters[0].title, "Rust Ecosystem");
        assert_eq!(validated.clusters[1].title, "Python Ecosystem");
    }

    #[test]
    fn validate_clusters_rejects_empty_title() {
        let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
        let response = ClusterResponse {
            clusters: vec![ClusterAssignment {
                title: "  ".to_string(),
                leaf_slugs: vec!["a".to_string(), "b".to_string()],
            }],
        };
        let mut warnings = Vec::new();
        let result = cluster::validate_clusters(&response, &leaves, &mut warnings);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty title"),
            "expected empty title error, got: {}",
            err
        );
    }

    #[test]
    fn validate_clusters_rejects_duplicate_title() {
        let leaves = vec![
            loaded_leaf("a", "A"),
            loaded_leaf("b", "B"),
            loaded_leaf("c", "C"),
            loaded_leaf("d", "D"),
        ];
        let response = ClusterResponse {
            clusters: vec![
                ClusterAssignment {
                    title: "Same".to_string(),
                    leaf_slugs: vec!["a".to_string(), "b".to_string()],
                },
                ClusterAssignment {
                    title: "Same".to_string(),
                    leaf_slugs: vec!["c".to_string(), "d".to_string()],
                },
            ],
        };
        let mut warnings = Vec::new();
        let result = cluster::validate_clusters(&response, &leaves, &mut warnings);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("duplicate"),
            "expected duplicate error, got: {}",
            err
        );
    }

    #[test]
    fn validate_clusters_rejects_single_leaf_cluster() {
        let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
        let response = ClusterResponse {
            clusters: vec![ClusterAssignment {
                title: "Solo".to_string(),
                leaf_slugs: vec!["a".to_string()],
            }],
        };
        let mut warnings = Vec::new();
        let result = cluster::validate_clusters(&response, &leaves, &mut warnings);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("at least 2"),
            "expected min-leaves error, got: {}",
            err
        );
    }

    #[test]
    fn validate_clusters_repairs_unknown_leaf_and_drops_cluster_if_below_2() {
        let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
        let response = ClusterResponse {
            clusters: vec![ClusterAssignment {
                title: "Concept".to_string(),
                leaf_slugs: vec!["a".to_string(), "nonexistent".to_string()],
            }],
        };
        let mut warnings = Vec::new();
        let result = cluster::validate_clusters(&response, &leaves, &mut warnings);
        // Unknown leaf ref dropped → cluster has 1 leaf → cluster dropped → no clusters → error.
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("repaired away"),
            "expected repaired-away error, got: {}",
            err
        );
        // Should have warned about the unknown ref and the cluster drop.
        assert!(
            warnings.iter().any(|w| w.contains("unknown leaf")),
            "should warn about unknown leaf"
        );
    }

    #[test]
    fn validate_clusters_repairs_unknown_leaf_keeps_cluster_if_still_valid() {
        let leaves = vec![
            loaded_leaf("a", "A"),
            loaded_leaf("b", "B"),
            loaded_leaf("c", "C"),
        ];
        let response = ClusterResponse {
            clusters: vec![ClusterAssignment {
                title: "Concept".to_string(),
                leaf_slugs: vec!["a".to_string(), "b".to_string(), "nonexistent".to_string()],
            }],
        };
        let mut warnings = Vec::new();
        let result = cluster::validate_clusters(&response, &leaves, &mut warnings);
        // Unknown dropped, 2 remain → cluster survives.
        assert!(result.is_ok());
        let validated = result.unwrap();
        assert_eq!(validated.clusters.len(), 1);
        assert_eq!(validated.clusters[0].leaf_files.len(), 2);
        assert!(
            warnings.iter().any(|w| w.contains("unknown leaf")),
            "should warn about unknown leaf"
        );
    }

    #[test]
    fn validate_clusters_repairs_cross_cluster_duplicate() {
        let leaves = vec![
            loaded_leaf("a", "A"),
            loaded_leaf("b", "B"),
            loaded_leaf("c", "C"),
        ];
        let response = ClusterResponse {
            clusters: vec![
                ClusterAssignment {
                    title: "One".to_string(),
                    leaf_slugs: vec!["a".to_string(), "b".to_string()],
                },
                ClusterAssignment {
                    title: "Two".to_string(),
                    leaf_slugs: vec!["b".to_string(), "c".to_string()],
                },
            ],
        };
        let mut warnings = Vec::new();
        let result = cluster::validate_clusters(&response, &leaves, &mut warnings);
        // "b" kept in first cluster ("One"), dropped from second. Second has only "c" → below 2 → dropped.
        // First cluster survives with a,b.
        assert!(result.is_ok());
        let validated = result.unwrap();
        assert_eq!(validated.clusters.len(), 1);
        assert_eq!(validated.clusters[0].title, "One");
        assert_eq!(validated.clusters[0].leaf_files.len(), 2);
        assert!(
            warnings.iter().any(|w| w.contains("multiple clusters")),
            "should warn about cross-cluster duplicate"
        );
    }

    // ── incremental cluster validation ───────────────────────────────────

    #[test]
    fn validate_incremental_clusters_accepts_assignment_and_new_cluster() {
        let existing_branch = Branch {
            slug: Slug::generate("existing-concept", ""),
            file: "branch/existing-concept.md".to_string(),
            title: Title::parse("Existing Concept").unwrap(),
            created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            updated_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            leaves: vec![Slug::generate("old-leaf", "")],
        };
        let manifest = Manifest {
            tree: TreeMeta {
                name: "test".to_string(),
                created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                last_compiled_at: Some(Timestamp::parse("2026-01-02T00:00:00Z").unwrap()),
            },
            leaves: vec![
                leaf_record(
                    "old-leaf",
                    "old-leaf.md",
                    "Old Leaf",
                    "2026-01-01T00:00:00Z",
                ),
                leaf_record("new-1", "new-1.md", "New One", "2026-01-03T00:00:00Z"),
                leaf_record("new-2", "new-2.md", "New Two", "2026-01-03T00:00:00Z"),
                leaf_record("new-3", "new-3.md", "New Three", "2026-01-03T00:00:00Z"),
                leaf_record("new-4", "new-4.md", "New Four", "2026-01-03T00:00:00Z"),
            ],
            branches: vec![existing_branch],
        };
        let leaves = vec![
            loaded_leaf("new-1", "New One"),
            loaded_leaf("new-2", "New Two"),
            loaded_leaf("new-3", "New Three"),
            loaded_leaf("new-4", "New Four"),
        ];
        let response = IncrementalClusterResponse {
            assignments: vec![BranchAssignment {
                branch_slug: "existing-concept".to_string(),
                leaf_slugs: vec!["new-1".to_string(), "new-2".to_string()],
            }],
            new_clusters: vec![ClusterAssignment {
                title: "Brand New Concept".to_string(),
                leaf_slugs: vec!["new-3".to_string(), "new-4".to_string()],
            }],
        };
        let mut warnings = Vec::new();
        let result =
            cluster::validate_incremental_clusters(&response, &manifest, &leaves, &mut warnings);
        assert!(
            result.is_ok(),
            "valid incremental clusters should pass: {:?}",
            result.err()
        );
        let validated = result.unwrap();
        assert_eq!(validated.clusters.len(), 2);
        // First cluster is the assignment to existing branch.
        assert!(validated.clusters[0].is_existing_branch());
        assert_eq!(
            validated.clusters[0].existing_branch_slug,
            "existing-concept"
        );
        // Second cluster is the new cluster.
        assert!(!validated.clusters[1].is_existing_branch());
    }

    #[test]
    fn validate_incremental_clusters_repairs_unknown_branch() {
        let manifest = Manifest {
            tree: TreeMeta {
                name: "test".to_string(),
                created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                last_compiled_at: Some(Timestamp::parse("2026-01-02T00:00:00Z").unwrap()),
            },
            leaves: vec![
                leaf_record("new-1", "new-1.md", "New One", "2026-01-03T00:00:00Z"),
                leaf_record("new-2", "new-2.md", "New Two", "2026-01-03T00:00:00Z"),
            ],
            branches: vec![],
        };
        let leaves = vec![
            loaded_leaf("new-1", "New One"),
            loaded_leaf("new-2", "New Two"),
        ];
        let response = IncrementalClusterResponse {
            assignments: vec![BranchAssignment {
                branch_slug: "nonexistent".to_string(),
                leaf_slugs: vec!["new-1".to_string(), "new-2".to_string()],
            }],
            new_clusters: vec![],
        };
        let mut warnings = Vec::new();
        let result =
            cluster::validate_incremental_clusters(&response, &manifest, &leaves, &mut warnings);
        // Unknown branch → assignment dropped. No clusters remain → error.
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("repaired away"),
            "expected repaired-away error, got: {}",
            err
        );
        assert!(
            warnings.iter().any(|w| w.contains("unknown branch")),
            "should warn about unknown branch"
        );
    }

    #[test]
    fn validate_incremental_clusters_rejects_new_cluster_title_collision() {
        use super::super::*;
        let existing_branch = Branch {
            slug: Slug::generate("existing", ""),
            file: "branch/existing.md".to_string(),
            title: Title::parse("Existing").unwrap(),
            created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            updated_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            leaves: vec![Slug::generate("old", "")],
        };
        let manifest = Manifest {
            tree: TreeMeta {
                name: "test".to_string(),
                created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                last_compiled_at: Some(Timestamp::parse("2026-01-02T00:00:00Z").unwrap()),
            },
            leaves: vec![
                leaf_record("new-1", "new-1.md", "New One", "2026-01-03T00:00:00Z"),
                leaf_record("new-2", "new-2.md", "New Two", "2026-01-03T00:00:00Z"),
            ],
            branches: vec![existing_branch],
        };
        let leaves = vec![
            loaded_leaf("new-1", "New One"),
            loaded_leaf("new-2", "New Two"),
        ];
        let response = IncrementalClusterResponse {
            assignments: vec![],
            new_clusters: vec![ClusterAssignment {
                title: "Existing".to_string(),
                leaf_slugs: vec!["new-1".to_string(), "new-2".to_string()],
            }],
        };
        let mut warnings = Vec::new();
        let result =
            cluster::validate_incremental_clusters(&response, &manifest, &leaves, &mut warnings);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("collides"),
            "expected collision error, got: {}",
            err
        );
    }

    // ── threshold selection ──────────────────────────────────────────────

    #[test]
    fn two_stage_threshold_values() {
        assert_eq!(super::super::TWO_STAGE_FULL_THRESHOLD, 40);
        assert_eq!(super::super::TWO_STAGE_INCREMENTAL_THRESHOLD, 15);
    }

    // ── stage-2 body-only schema ────────────────────────────────────────

    #[test]
    fn parse_stage2_response_valid_constructs_branch() {
        let response = Ok(r#"{"title": "Concept Name", "body": "Synthesized body."}"#.to_string());
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let mut warnings = Vec::new();
        let branch = super::super::cluster::parse_stage2_response(
            &response,
            "test-cluster",
            &leaf_files,
            &mut warnings,
        )
        .expect("valid response should parse");
        assert_eq!(branch.title, "Concept Name");
        assert_eq!(branch.body, "Synthesized body.");
        // Membership comes from cluster, not from the response.
        assert_eq!(branch.leaves, leaf_files);
    }

    #[test]
    fn parse_stage2_response_rejects_malformed_json() {
        let response = Ok("not json".to_string());
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let mut warnings = Vec::new();
        let result = super::super::cluster::parse_stage2_response(
            &response,
            "test-cluster",
            &leaf_files,
            &mut warnings,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid stage-2"),
            "expected parse error, got: {}",
            err
        );
    }

    #[test]
    fn parse_stage2_response_rejects_empty_title() {
        let response = Ok(r#"{"title": "  ", "body": "Some body."}"#.to_string());
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let mut warnings = Vec::new();
        let result = super::super::cluster::parse_stage2_response(
            &response,
            "test-cluster",
            &leaf_files,
            &mut warnings,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty title"),
            "expected empty title error, got: {}",
            err
        );
    }

    #[test]
    fn parse_stage2_response_rejects_empty_body() {
        let response = Ok(r#"{"title": "Title", "body": "  "}"#.to_string());
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let mut warnings = Vec::new();
        let result = super::super::cluster::parse_stage2_response(
            &response,
            "test-cluster",
            &leaf_files,
            &mut warnings,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("empty body"),
            "expected empty body error, got: {}",
            err
        );
    }

    #[test]
    fn parse_stage2_response_membership_from_cluster_not_response() {
        // Response includes "leaves" field but it is ignored (deny_unknown_fields
        // would reject it — so this tests the schema rejects extra fields).
        let response =
            Ok(r#"{"title": "Concept", "body": "Body.", "leaves": ["hacked.md"]}"#.to_string());
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let mut warnings = Vec::new();
        let result = super::super::cluster::parse_stage2_response(
            &response,
            "test-cluster",
            &leaf_files,
            &mut warnings,
        );
        // deny_unknown_fields should reject the extra "leaves" field.
        assert!(result.is_err(), "should reject unknown fields");
    }

    #[test]
    fn parse_stage2_response_llm_error_propagates() {
        let response: Result<String, super::super::CompileError> =
            Err(super::super::CompileError::Truncated);
        let leaf_files = vec!["a.md".to_string(), "b.md".to_string()];
        let mut warnings = Vec::new();
        let result = super::super::cluster::parse_stage2_response(
            &response,
            "test-cluster",
            &leaf_files,
            &mut warnings,
        );
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("truncated") || err.contains("LLM call failed"),
            "expected LLM error propagation, got: {}",
            err
        );
    }
}
