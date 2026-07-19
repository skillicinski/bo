use super::*;
use crate::cli::compile::types::{CompileError, CompileRunMode};
use crate::domain::slug::Slug;
use crate::domain::state::{TreeMetadata, TreeState};
use crate::domain::{Branch, Leaf, Timestamp, Title, Url};
use crate::engine::config::SeededConfig;
use crate::engine::llm::{
    AgentMessage, AgentResponse, FinishReason, LlmError, LlmProvider, LlmResponse, Model, Provider,
    ProviderSchema, ToolSchema,
};
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

fn write_state(dir: &Path, state: &TreeState) {
    let tree = crate::domain::tree::Tree::from_config(&crate::domain::tree::TreeConfig {
        path: dir.to_path_buf(),
        name: "test-tree".to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
    });
    crate::engine::state::write(&crate::domain::tree::state_path(&tree.path), state).unwrap();
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

fn fresh_state(name: &str, created_at: &str, last_compiled_at: Option<&str>) -> TreeState {
    TreeState {
        tree: TreeMetadata {
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

fn loaded_leaf(slug: &str, title: &str) -> crate::cli::compile::plan::LoadedLeaf {
    crate::cli::compile::plan::LoadedLeaf {
        slug: slug.to_string(),
        filename: format!("{}.md", slug),
        title: title.to_string(),
        summary: None,
        body: format!("body of {}", title),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

struct ScriptedAgentProvider {
    responses: Vec<AgentResponse>,
    calls: AtomicUsize,
    messages: Mutex<Vec<Vec<AgentMessage>>>,
    tool_schemas: Mutex<Option<Vec<ToolSchema>>>,
}

impl ScriptedAgentProvider {
    fn new(responses: Vec<AgentResponse>) -> Self {
        Self {
            responses,
            calls: AtomicUsize::new(0),
            messages: Mutex::new(Vec::new()),
            tool_schemas: Mutex::new(None),
        }
    }

    fn messages(&self) -> Vec<Vec<AgentMessage>> {
        self.messages
            .lock()
            .expect("scripted messages poisoned")
            .clone()
    }

    fn tool_schemas(&self) -> Option<Vec<ToolSchema>> {
        self.tool_schemas
            .lock()
            .expect("scripted tool schemas poisoned")
            .clone()
    }
}

#[async_trait]
impl LlmProvider for ScriptedAgentProvider {
    async fn complete(
        &self,
        _: &[crate::engine::llm::Message],
        _: &str,
        _: u32,
        _: Option<&ProviderSchema>,
        _: bool,
    ) -> Result<LlmResponse, LlmError> {
        unreachable!("agent compile tests only use complete_with_tools")
    }

    async fn complete_with_tools(
        &self,
        messages: &[AgentMessage],
        _: &str,
        _: u32,
        tool_schemas: &[ToolSchema],
        _: bool,
    ) -> Result<AgentResponse, LlmError> {
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
            .unwrap_or_else(|| AgentResponse {
                content: Some(String::new()),
                reasoning_content: None,
                tool_calls: Vec::new(),
                finish_reason: FinishReason::Stop,
                usage: None,
            }))
    }
}

fn agent_tool_response(id: &str, name: &str, arguments: &str) -> AgentResponse {
    AgentResponse {
        content: None,
        reasoning_content: None,
        tool_calls: vec![crate::engine::llm::ToolCall {
            id: id.to_string(),
            name: name.to_string(),
            arguments: arguments.to_string(),
        }],
        finish_reason: FinishReason::Other("tool_calls".to_string()),
        usage: None,
    }
}

fn agent_model() -> Model {
    Model::parse("deepseek-v4-flash", Provider::Deepseek).unwrap()
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

fn setup_incremental_tree(
    dir: &Path,
) -> (
    SeededConfig,
    TreeState,
    Vec<crate::cli::compile::plan::LoadedLeaf>,
) {
    let mut state = fresh_state("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));
    state.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-01-15T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-c",
        "leaf-c.md",
        "Leaf C",
        "2026-01-05T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-d",
        "leaf-d.md",
        "Leaf D",
        "2026-01-05T00:00:00Z",
    ));
    state.branches.push(branch_record(
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

    write_state(dir, &state);

    let cfg = seeded_config(dir);
    let loaded = vec![
        loaded_leaf("leaf-a", "Leaf A"),
        loaded_leaf("leaf-b", "Leaf B"),
        loaded_leaf("leaf-c", "Leaf C"),
        loaded_leaf("leaf-d", "Leaf D"),
    ];
    (cfg, state, loaded)
}

fn find_tool_result(
    messages: &[Vec<AgentMessage>],
    tool_call_id: &str,
) -> Option<serde_json::Value> {
    let content = messages.iter().find_map(|turn_messages| {
        turn_messages.iter().find_map(|message| match message {
            AgentMessage::Tool(result) if result.tool_call_id == tool_call_id => {
                Some(result.content.as_str())
            }
            _ => None,
        })
    })?;
    serde_json::from_str(content).ok()
}

// ── tests ─────────────────────────────────────────────────────────────────────

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
        let (cfg, state, loaded) = setup_incremental_tree(dir.path());
        let provider = ScriptedAgentProvider::new(vec![agent_tool_response(
            "submit",
            "submit_compile",
            &submission,
        )]);
        let model = agent_model();

        run_agent_dry_run(&cfg, &provider, &model, &state, &loaded, run_mode)
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
                AgentMessage::System(s) => Some(s.as_str()),
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
    let (cfg, state, loaded) = setup_incremental_tree(dir.path());
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
        &state,
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
        let (cfg, state, loaded) = setup_incremental_tree(dir.path());
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
            &state,
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
        let (cfg, state, loaded) = setup_incremental_tree(dir.path());
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
            &state,
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

#[test]
fn agent_incremental_branch_identifier_round_trips_from_list_branches() {
    let dir = TempDir::new().unwrap();
    let (cfg, state, loaded) = setup_incremental_tree(dir.path());
    let submission =
        incremental_update_submission("branch/existing", &["leaf-c", "leaf-d", "leaf-a"]);
    let provider = ScriptedAgentProvider::new(vec![
        agent_tool_response("list_branches", "list_branches", "{}"),
        agent_tool_response("submit", "submit_compile", &submission),
    ]);
    let model = agent_model();

    let (plan, stats, _) = run_agent_dry_run(
        &cfg,
        &provider,
        &model,
        &state,
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
            AgentMessage::Tool(result) if result.tool_call_id == "list_branches" => {
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
    let (cfg, state, loaded) = setup_incremental_tree(dir.path());
    let submission = incremental_update_submission("existing", &["leaf-c", "leaf-d", "leaf-a"]);
    let provider = ScriptedAgentProvider::new(vec![agent_tool_response(
        "submit",
        "submit_compile",
        &submission,
    )]);
    let model = agent_model();

    let (plan, _, _) = run_agent_dry_run(
        &cfg,
        &provider,
        &model,
        &state,
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

    let err = crate::cli::compile::parse::parse_and_validate_incremental_with_input_size(
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
        let (cfg, state, loaded) = setup_incremental_tree(dir.path());
        let submission =
            incremental_update_submission("existing", &["leaf-c", "leaf-d", identifier]);
        let provider = ScriptedAgentProvider::new(vec![agent_tool_response(
            "submit",
            "submit_compile",
            &submission,
        )]);
        let model = agent_model();

        let err = run_agent_dry_run(
            &cfg,
            &provider,
            &model,
            &state,
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
