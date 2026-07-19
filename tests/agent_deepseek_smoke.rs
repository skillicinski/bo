// Live DeepSeek smoke tests for the agent compile path.
//
// Requires DEEPSEEK_API_KEY in the environment.
// Marked `#[ignore]` so CI stays key-free.
//
// Run manually:
//   DEEPSEEK_API_KEY=sk-... cargo test --test agent_deepseek_smoke -- --ignored

mod common;

use bo::cli::compile::{self, CompileDryRunOutcome, CompileOptions};
use bo::engine::llm::{self, LlmProvider, Model, Provider};

fn run_dry_run_test(model_id: &str, provider: Box<dyn LlmProvider>, model: &Model) {
    let dir = common::setup_fixture_collection();
    let cfg = common::seeded_config(dir.path(), Provider::Deepseek, model_id);

    // Snapshot tree before
    let before = common::snapshot_tree(dir.path());

    let CompileDryRunOutcome { result, .. } = compile::run_compile_dry_run_with_provider(
        &cfg,
        CompileOptions {
            all: false,
            agent: true,
            dry_run: true,
        },
        provider.as_ref(),
        model,
    );

    let preview = result.unwrap_or_else(|e| panic!("dry-run failed: {e:?}"));

    assert_eq!(preview.status, "preview", "expected preview status");
    assert!(preview.agent, "expected agent=true");
    assert!(
        preview.turns >= 2,
        "expected >=2 agent turns, got {}",
        preview.turns
    );
    assert!(
        preview.tool_calls >= 2,
        "expected >=2 tool calls, got {}",
        preview.tool_calls
    );
    assert!(!preview.branches.is_empty(), "expected non-empty branches");
    assert!(preview.state_unchanged, "expected state_unchanged=true");
    assert_eq!(preview.model, model_id, "unexpected model in preview");

    // Snapshot tree after
    let after = common::snapshot_tree(dir.path());

    assert_eq!(
        before, after,
        "tree dir changed — dry-run wrote bytes when it should not have"
    );
}

// ── live tests ───────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires DEEPSEEK_API_KEY (live)"]
fn flash_non_thinking_completes_two_tool_turns() {
    let Ok(api_key) = std::env::var("DEEPSEEK_API_KEY") else {
        eprintln!("skipped: DEEPSEEK_API_KEY not set");
        return;
    };
    let model_id = "deepseek-v4-flash";
    let provider = llm::create_provider(Provider::Deepseek, &api_key, None)
        .expect("failed to create DeepSeek provider");
    let model = Model::parse(model_id, Provider::Deepseek).expect("failed to parse model");

    run_dry_run_test(model_id, provider, &model);
}

#[test]
#[ignore = "requires DEEPSEEK_API_KEY (live)"]
fn pro_thinking_completes_two_tool_turns() {
    let Ok(api_key) = std::env::var("DEEPSEEK_API_KEY") else {
        eprintln!("skipped: DEEPSEEK_API_KEY not set");
        return;
    };
    let model_id = "deepseek-v4-pro";
    let provider = llm::create_provider(Provider::Deepseek, &api_key, None)
        .expect("failed to create DeepSeek provider");
    let model = Model::parse(model_id, Provider::Deepseek).expect("failed to parse model");

    run_dry_run_test(model_id, provider, &model);
}
