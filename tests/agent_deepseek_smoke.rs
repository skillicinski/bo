// Live DeepSeek smoke tests for the agent compile path.
//
// Requires DEEPSEEK_API_KEY in the environment.
// Marked `#[ignore]` so CI stays key-free.
//
// Run manually:
//   DEEPSEEK_API_KEY=sk-... cargo test --test agent_deepseek_smoke -- --ignored

use std::fs;

use bo::cli::compile::{self, CompileDryRunOutcome, CompileOptions};
use bo::domain::manifest::{Manifest, TreeMeta};
use bo::domain::Leaf;
use bo::domain::Timestamp;
use bo::domain::{Title, Url};
use bo::engine::config::SeededConfig;
use bo::engine::llm::{self, LlmProvider, Model, Provider};

struct FixtureDoc {
    file: &'static str,
    title: &'static str,
    url: &'static str,
    body: &'static str,
}

const FIXTURE_DOCS: &[FixtureDoc] = &[
    FixtureDoc {
        file: "rust-ownership.md",
        title: "Rust Ownership",
        url: "https://example.com/rust-ownership",
        body: "Rust's ownership model makes memory safety a compile-time property. Borrowing and lifetimes let programs share references without a garbage collector while still controlling resource cleanup precisely.",
    },
    FixtureDoc {
        file: "memory-safety.md",
        title: "Memory Safety",
        url: "https://example.com/memory-safety",
        body: "Memory safety matters in systems programming because pointer mistakes can become security bugs. Rust uses ownership, borrowing, and lifetimes to prevent dangling references and data races before runtime.",
    },
    FixtureDoc {
        file: "safe-concurrency.md",
        title: "Safe Concurrency",
        url: "https://example.com/safe-concurrency",
        body: "Safe concurrency depends on clear ownership of shared state. Rust's type system prevents data races by enforcing borrowing rules across threads and synchronisation boundaries.",
    },
    FixtureDoc {
        file: "zero-cost-abstractions.md",
        title: "Zero-Cost Abstractions",
        url: "https://example.com/zero-cost-abstractions",
        body: "Zero-cost abstractions allow high-level APIs without runtime penalties. In Rust, ownership and static dispatch let systems code remain expressive while preserving predictable memory and performance behaviour.",
    },
];

fn setup_fixture_collection() -> tempfile::TempDir {
    let dir = tempfile::TempDir::new().unwrap();
    let bo_dir = dir.path().join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    let mut leaves = Vec::new();

    for doc in FIXTURE_DOCS {
        let title: Option<Title> = Title::parse(doc.title).ok();
        let url = Url::parse(doc.url).unwrap();
        let ts = Timestamp::parse("2025-06-01T10:00:00Z").unwrap();
        let content = bo::domain::leaf::format_content(title.as_ref(), &url, &ts, doc.body);
        fs::write(dir.path().join(doc.file), content).unwrap();

        leaves.push(Leaf {
            slug: bo::domain::Slug::parse(doc.file.trim_end_matches(".md")).unwrap(),
            file: doc.file.to_string(),
            title,
            url,
            collected_at: Timestamp::parse("2025-06-01T10:00:00Z").unwrap(),
            summary: None,
        });
    }

    bo::engine::manifest::write(
        &bo_dir.join("manifest.json"),
        &Manifest {
            tree: TreeMeta {
                name: "deepseek-smoke-fixture".to_string(),
                created_at: Timestamp::parse("2025-06-01T09:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves,
            branches: Vec::new(),
        },
    )
    .unwrap();

    dir
}

fn make_deepseek_config(output_dir: &std::path::Path) -> SeededConfig {
    SeededConfig::new(
        bo::engine::config::Config {
            provider: Provider::Deepseek,
            model: "deepseek-v4-flash".to_string(),
            compile_model: None,
            base_url: None,
            tree: None,
        },
        bo::domain::tree::TreeConfig {
            path: output_dir.to_path_buf(),
            name: "deepseek-smoke".to_string(),
            created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        },
    )
}

/// Recursively snapshot a directory: map of relative path -> file content.
fn snapshot_dir(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    let mut entries = Vec::new();
    collect_files(dir, dir, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn collect_files(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<(String, Vec<u8>)>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let rel = path
            .strip_prefix(root)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else {
            let content = fs::read(&path).unwrap();
            out.push((rel, content));
        }
    }
}

fn run_dry_run_test(model_id: &str, provider: Box<dyn LlmProvider>, model: &Model) {
    let dir = setup_fixture_collection();
    let cfg = make_deepseek_config(dir.path());

    // Snapshot tree before
    let before = snapshot_dir(dir.path());

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
    assert!(
        preview.manifest_unchanged,
        "expected manifest_unchanged=true"
    );
    assert_eq!(preview.model, model_id, "unexpected model in preview");

    // Snapshot tree after
    let after = snapshot_dir(dir.path());

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
