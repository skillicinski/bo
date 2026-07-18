// Integration tests for `bo compile`.
//
// All tests use injectable mock LLM providers and run in CI without API keys.
// Former live-API tests were migrated/dropped per #131 — their unique
// assertions were folded into the mock-based tests below.

use bo::domain::Timestamp;
use bo::domain::{Title, Url};
use std::fs;

use bo::cli::compile;
use bo::domain::manifest::{Manifest, TreeMeta};
use bo::domain::{Branch, Leaf};
use bo::engine::config::SeededConfig;
use bo::engine::pending;

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

/// Build a small synthetic tree in a temp directory and return the path.
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
                name: "compile-fixture".to_string(),
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

fn make_config(output_dir: &std::path::Path) -> SeededConfig {
    SeededConfig::new(
        bo::engine::config::Config {
            provider: bo::engine::llm::Provider::OpenAI,
            model: "gpt-4o-mini".to_string(), // cheaper model for tests
            compile_model: None,
            base_url: None,
            tree: None,
        },
        bo::domain::tree::TreeConfig {
            path: output_dir.to_path_buf(),
            name: "test-tree".to_string(),
            created_at: bo::domain::Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        },
    )
}

// ── crash recovery integration tests ─────────────────────────────────────────

#[test]
fn crash_mid_compile_rollback_cleans_staged_files() {
    // Simulate: compile started, pending.json written, staged .tmp exists,
    // but manifest was NOT updated (crash before commit).
    // Next invocation should rollback: delete .tmp, clear pending.
    let dir = setup_fixture_collection();
    let tree_dir = dir.path();
    let bo_dir = tree_dir.join(".bo");

    // Mark all leaves as compiled so compile returns noop after recovery
    let manifest_path = bo_dir.join("manifest.json");
    let mut m = bo::engine::manifest::read(&manifest_path).unwrap();
    m.tree.last_compiled_at = Some(Timestamp::parse("2099-01-01T00:00:00Z").unwrap());
    bo::engine::manifest::write(&manifest_path, &m).unwrap();

    // Record manifest hash before "crash"
    let manifest_hash = bo::engine::pending::manifest_hash(tree_dir).unwrap();

    // Write a staged .tmp file (simulating a branch about to be written)
    fs::create_dir_all(tree_dir.join("branch")).unwrap();
    let staged_content = b"# Fake Branch\n\nThis should be rolled back.\n";
    let staged_path = tree_dir.join("branch/fake-branch.md.tmp");
    fs::write(&staged_path, staged_content).unwrap();

    // Write pending.json with a dead PID and old timestamp (not a live lock)
    let pending = bo::engine::pending::PendingOperation {
        op: bo::engine::pending::OpKind::Compile {
            mode: bo::engine::pending::CompileMode::Full,
        },
        started_at: "2020-01-01T00:00:00Z".to_string(),
        pid: 99999,
        pre_manifest_hash: manifest_hash,
        writes: vec![bo::engine::pending::PendingWrite {
            path: "branch/fake-branch.md".to_string(),
            content_hash: bo::engine::pending::content_hash(staged_content),
        }],
        deletes: vec![],
    };
    let pending_path = bo_dir.join("pending.json");
    bo::engine::pending::write(&pending_path, &pending).unwrap();

    // Now run compile — should detect stale pending, rollback, then proceed normally
    let cfg = make_config(tree_dir);
    let outcome = compile::run_compile_with_options(&cfg, Default::default());

    // The staged file should be gone (rolled back)
    assert!(
        !staged_path.exists(),
        "staged .tmp should be deleted on rollback"
    );
    // pending.json should be cleared
    assert!(
        !pending_path.exists(),
        "pending.json should be cleared after recovery"
    );
    // Compile itself should succeed (noop or compiled depending on state)
    assert!(
        outcome.result.is_ok(),
        "compile should succeed after recovery: {:?}",
        outcome.result.err()
    );
    // The recovery notice is collected as a diagnostic line, not printed below
    // the entry point.
    assert!(
        outcome
            .stderr_lines()
            .iter()
            .any(|l| l.contains("recovered")),
        "recovery notice should land as a diagnostic line: {:?}",
        outcome.stderr_lines()
    );
}

#[test]
fn crash_mid_compile_roll_forward_applies_staged_writes() {
    // Simulate: compile started, pending.json written, staged .tmp exists,
    // AND manifest WAS updated (crash after commit but before rename).
    // Next invocation should roll forward: rename .tmp to final, clear pending.
    let dir = setup_fixture_collection();
    let tree_dir = dir.path();
    let bo_dir = tree_dir.join(".bo");

    // Mark all leaves as compiled
    let manifest_path = bo_dir.join("manifest.json");
    let mut m = bo::engine::manifest::read(&manifest_path).unwrap();
    m.tree.last_compiled_at = Some(Timestamp::parse("2099-01-01T00:00:00Z").unwrap());
    bo::engine::manifest::write(&manifest_path, &m).unwrap();

    // Write a staged .tmp file
    fs::create_dir_all(tree_dir.join("branch")).unwrap();
    let branch_content =
        b"---\ntitle: Recovered Branch\n---\n\n# Recovered Branch\n\nThis was recovered.\n";
    let staged_path = tree_dir.join("branch/recovered-branch.md.tmp");
    let final_path = tree_dir.join("branch/recovered-branch.md");
    fs::write(&staged_path, branch_content).unwrap();

    // Use a DIFFERENT hash than current manifest (simulating that manifest was already committed)
    let fake_pre_hash = "deadbeef".to_string();

    let pending = bo::engine::pending::PendingOperation {
        op: bo::engine::pending::OpKind::Compile {
            mode: bo::engine::pending::CompileMode::Full,
        },
        started_at: "2020-01-01T00:00:00Z".to_string(),
        pid: 99999,
        pre_manifest_hash: fake_pre_hash,
        writes: vec![bo::engine::pending::PendingWrite {
            path: "branch/recovered-branch.md".to_string(),
            content_hash: bo::engine::pending::content_hash(branch_content),
        }],
        deletes: vec![],
    };
    let pending_path = bo_dir.join("pending.json");
    bo::engine::pending::write(&pending_path, &pending).unwrap();

    // Run compile — should roll forward
    let cfg = make_config(tree_dir);
    let result = compile::run_compile_with_options(&cfg, Default::default()).result;

    // The staged file should be renamed to final
    assert!(
        !staged_path.exists(),
        "staged .tmp should be gone after roll-forward"
    );
    assert!(
        final_path.exists(),
        "final file should exist after roll-forward"
    );
    let content = fs::read_to_string(&final_path).unwrap();
    assert!(content.contains("Recovered Branch"));
    // pending.json cleared
    assert!(!pending_path.exists(), "pending.json should be cleared");
    assert!(
        result.is_ok(),
        "compile should succeed after recovery: {:?}",
        result.err()
    );
}

#[test]
fn crash_mid_collect_rollback_leaves_tree_unchanged() {
    // Simulate: collect started, pending.json written for a collect op,
    // staged leaf .tmp exists, manifest NOT updated.
    // Next compile invocation should rollback the stale collect.
    let dir = setup_fixture_collection();
    let tree_dir = dir.path();
    let bo_dir = tree_dir.join(".bo");

    // Mark all leaves as compiled
    let manifest_path = bo_dir.join("manifest.json");
    let mut m = bo::engine::manifest::read(&manifest_path).unwrap();
    m.tree.last_compiled_at = Some(Timestamp::parse("2099-01-01T00:00:00Z").unwrap());
    bo::engine::manifest::write(&manifest_path, &m).unwrap();

    let manifest_hash = bo::engine::pending::manifest_hash(tree_dir).unwrap();

    // Staged leaf file from interrupted collect
    let staged_leaf =
        b"---\ntitle: Interrupted\nurl: https://example.com/interrupted\n---\n\nBody.\n";
    let staged_path = tree_dir.join("interrupted.md.tmp");
    fs::write(&staged_path, staged_leaf).unwrap();

    let pending = bo::engine::pending::PendingOperation {
        op: bo::engine::pending::OpKind::Collect {
            url: "https://example.com/interrupted".to_string(),
        },
        started_at: "2020-01-01T00:00:00Z".to_string(),
        pid: 99999,
        pre_manifest_hash: manifest_hash,
        writes: vec![bo::engine::pending::PendingWrite {
            path: "interrupted.md".to_string(),
            content_hash: bo::engine::pending::content_hash(staged_leaf),
        }],
        deletes: vec![],
    };
    bo::engine::pending::write(&bo_dir.join("pending.json"), &pending).unwrap();

    // Run compile — should recover (rollback) first, then proceed
    let cfg = make_config(tree_dir);
    let _result = compile::run_compile_with_options(&cfg, Default::default());

    // Staged file rolled back
    assert!(
        !staged_path.exists(),
        "staged collect .tmp should be rolled back"
    );
    assert!(
        !tree_dir.join("interrupted.md").exists(),
        "interrupted leaf should not appear"
    );
    // Manifest unchanged by the failed collect
    let _manifest_after = fs::read_to_string(bo_dir.join("manifest.json")).unwrap();
    // Note: compile may update manifest (last_compiled_at), so just verify no interrupted leaf
    let m = bo::engine::manifest::read(&bo_dir.join("manifest.json")).unwrap();
    assert!(!m
        .leaves
        .iter()
        .any(|leaf| leaf.slug.as_str() == "interrupted"));
    assert!(!bo_dir.join("pending.json").exists());
}

// ── canned-response integration tests (no API key required) ──────────────────

use async_trait::async_trait;

struct FakeLlmProvider {
    response: String,
}

#[async_trait]
impl bo::engine::llm::LlmProvider for FakeLlmProvider {
    async fn complete(
        &self,
        _messages: &[bo::engine::llm::Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&bo::engine::llm::NormalizedSchema>,
        _reasoning_disabled: bool,
    ) -> Result<bo::engine::llm::LlmResponse, bo::engine::llm::LlmError> {
        Ok(bo::engine::llm::LlmResponse {
            content: self.response.clone(),
            finish_reason: bo::engine::llm::FinishReason::Stop,
        })
    }
}

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Provider that records every system message and returns canned responses in order.
/// Uses interior mutability so it can be passed as `&dyn LlmProvider`.
struct CapturingProvider {
    system_prompts: Arc<Mutex<Vec<String>>>,
    responses: Arc<Mutex<VecDeque<String>>>,
}

impl CapturingProvider {
    fn new(responses: Vec<String>) -> Self {
        Self {
            system_prompts: Arc::new(Mutex::new(Vec::new())),
            responses: Arc::new(Mutex::new(VecDeque::from(responses))),
        }
    }
}

#[async_trait]
impl bo::engine::llm::LlmProvider for CapturingProvider {
    async fn complete(
        &self,
        messages: &[bo::engine::llm::Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&bo::engine::llm::NormalizedSchema>,
        _reasoning_disabled: bool,
    ) -> Result<bo::engine::llm::LlmResponse, bo::engine::llm::LlmError> {
        if let Some(msg) = messages.first() {
            self.system_prompts
                .lock()
                .unwrap()
                .push(msg.content.clone());
        }
        let response = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("CapturingProvider: no more canned responses");
        Ok(bo::engine::llm::LlmResponse {
            content: response,
            finish_reason: bo::engine::llm::FinishReason::Stop,
        })
    }
}

fn compile_model() -> bo::engine::llm::Model {
    bo::engine::llm::Model::parse("gpt-4.1", bo::engine::llm::Provider::OpenAI).unwrap()
}

#[test]
fn compile_full_with_canned_response_creates_branches() {
    let dir = setup_fixture_collection();
    let cfg = make_config(dir.path());
    let model = compile_model();
    let started_at = Timestamp::now();

    // Canned response: two branches, each covering two leaves.
    let canned = serde_json::json!({
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
    let provider = FakeLlmProvider {
        response: canned.to_string(),
    };

    let result = compile::run_compile_with_provider_started_at(
        &cfg,
        Default::default(),
        &provider,
        &model,
        &started_at,
        Vec::new(),
        &bo::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap(),
        &[], // ponytail: Full mode doesn't use new_leaf_slugs
        &pending::manifest_hash(dir.path()).unwrap(),
        &mut Vec::new(),
    );

    let compile_result = result.unwrap();
    assert_eq!(compile_result.status, "compiled");
    assert_eq!(compile_result.branches.len(), 2);

    // Verify branch files were written to disk
    let branches_dir = dir.path().join("branch");
    assert!(branches_dir.exists());

    let branch_a = branches_dir.join("rust-memory-safety.md");
    assert!(branch_a.exists(), "missing branch file: {:?}", branch_a);
    let content = fs::read_to_string(&branch_a).unwrap();
    assert!(content.contains("Rust Memory Safety"));
    assert!(content.contains("rust-ownership.md"));
    assert!(content.contains("memory-safety.md"));

    // Frontmatter schema assertions (migrated from dropped live tests)
    let (mapping, body) = bo::domain::frontmatter::parse(&content).unwrap();
    assert!(
        mapping.get("title").and_then(|v| v.as_str()).is_some(),
        "branch missing 'title' in frontmatter"
    );
    assert!(
        mapping.get("created_at").and_then(|v| v.as_str()).is_some(),
        "branch missing 'created_at' in frontmatter"
    );
    assert!(
        mapping.get("updated_at").and_then(|v| v.as_str()).is_some(),
        "branch missing 'updated_at' in frontmatter"
    );
    assert!(
        mapping.get("leaves").is_some(),
        "branch missing 'leaves' in frontmatter"
    );
    assert!(!body.trim().is_empty(), "branch body is empty");

    // No index store (migrated from dropped live test)
    assert!(
        !dir.path().join(".bo/index.jsonl").exists(),
        "compile must not write .bo/index.jsonl"
    );

    let branch_b = branches_dir.join("systems-design-in-rust.md");
    assert!(branch_b.exists(), "missing branch file: {:?}", branch_b);
    let content_b = fs::read_to_string(&branch_b).unwrap();
    assert!(content_b.contains("Systems Design in Rust"));

    // Manifest updated with branches
    let m = bo::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.branches.len(), 2);
    assert!(m.tree.last_compiled_at.is_some());
}

#[test]
fn compile_incremental_with_canned_response_updates_existing_branches() {
    // Build a tree with an existing compile: 2 leaves, 1 branch, last_compiled_at set.
    // Then add 2 new leaves with later collected_at.
    let dir = tempfile::TempDir::new().unwrap();
    let bo_dir = dir.path().join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    fs::create_dir_all(dir.path().join("branch")).unwrap();

    let ts_old = Timestamp::parse("2025-06-01T10:00:00Z").unwrap();
    let ts_last_compile = Timestamp::parse("2025-06-02T00:00:00Z").unwrap();
    let ts_new = Timestamp::parse("2025-06-03T10:00:00Z").unwrap();

    // Two existing leaves (already compiled)
    for (file, title, body) in [
        (
            "ownership.md",
            "Ownership",
            "Rust ownership ensures memory safety at compile time.",
        ),
        (
            "borrowing.md",
            "Borrowing",
            "Borrowing allows shared references without GC.",
        ),
    ] {
        let t = Title::parse(title).unwrap();
        let u = Url::parse(&format!("https://example.com/{}", file)).unwrap();
        let content = bo::domain::leaf::format_content(Some(&t), &u, &ts_old, body);
        fs::write(dir.path().join(file), content).unwrap();
    }

    // Two new leaves (collected after last compile)
    for (file, title, body) in [
        (
            "lifetimes.md",
            "Lifetimes",
            "Lifetimes let the compiler verify references.",
        ),
        (
            "traits.md",
            "Traits",
            "Traits define shared behaviour across types.",
        ),
    ] {
        let t = Title::parse(title).unwrap();
        let u = Url::parse(&format!("https://example.com/{}", file)).unwrap();
        let content = bo::domain::leaf::format_content(Some(&t), &u, &ts_new, body);
        fs::write(dir.path().join(file), content).unwrap();
    }

    // Write existing branch file on disk (needed for incremental prompt)
    fs::write(
        dir.path().join("branch/memory-model.md"),
        "---\ntitle: Memory Model\ncreated_at: 2025-06-02T00:00:00Z\nupdated_at: 2025-06-02T00:00:00Z\nleaves:\n  - ownership\n  - borrowing\n---\n\n# Memory Model\n\nOriginal body.\n",
    )
    .unwrap();

    bo::engine::manifest::write(
        &bo_dir.join("manifest.json"),
        &Manifest {
            tree: TreeMeta {
                name: "incremental-fixture".to_string(),
                created_at: ts_old.clone(),
                last_compiled_at: Some(ts_last_compile.clone()),
            },
            leaves: vec![
                Leaf {
                    slug: bo::domain::Slug::parse("ownership").unwrap(),
                    file: "ownership.md".to_string(),
                    title: Some(Title::parse("Ownership").unwrap()),
                    url: Url::parse("https://example.com/ownership.md").unwrap(),
                    collected_at: ts_old.clone(),
                    summary: None,
                },
                Leaf {
                    slug: bo::domain::Slug::parse("borrowing").unwrap(),
                    file: "borrowing.md".to_string(),
                    title: Some(Title::parse("Borrowing").unwrap()),
                    url: Url::parse("https://example.com/borrowing.md").unwrap(),
                    collected_at: ts_old.clone(),
                    summary: None,
                },
                Leaf {
                    slug: bo::domain::Slug::parse("lifetimes").unwrap(),
                    file: "lifetimes.md".to_string(),
                    title: Some(Title::parse("Lifetimes").unwrap()),
                    url: Url::parse("https://example.com/lifetimes.md").unwrap(),
                    collected_at: ts_new.clone(),
                    summary: None,
                },
                Leaf {
                    slug: bo::domain::Slug::parse("traits").unwrap(),
                    file: "traits.md".to_string(),
                    title: Some(Title::parse("Traits").unwrap()),
                    url: Url::parse("https://example.com/traits.md").unwrap(),
                    collected_at: ts_new.clone(),
                    summary: None,
                },
            ],
            branches: vec![Branch {
                slug: bo::domain::Slug::parse("memory-model").unwrap(),
                file: "branch/memory-model.md".to_string(),
                title: Title::parse("Memory Model").unwrap(),
                created_at: ts_last_compile.clone(),
                updated_at: ts_last_compile.clone(),
                leaves: vec![
                    bo::domain::Slug::parse("ownership").unwrap(),
                    bo::domain::Slug::parse("borrowing").unwrap(),
                ],
            }],
        },
    )
    .unwrap();

    let cfg = make_config(dir.path());
    let model = compile_model();
    let started_at = Timestamp::now();

    // Canned incremental response: update existing branch and create a new one.
    let canned = serde_json::json!({
        "updated_branches": [
            {
                "slug": "memory-model",
                "title": "Memory Model",
                "body": "# Memory Model\n\nUpdated with lifetimes.",
                "leaves": ["ownership.md", "borrowing.md", "lifetimes.md"]
            }
        ],
        "new_branches": [
            {
                "title": "Type System",
                "body": "# Type System\n\nTraits enable polymorphism.",
                "leaves": ["traits.md", "ownership.md"]
            }
        ]
    })
    .to_string();
    let provider = FakeLlmProvider { response: canned };

    let m = bo::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    let new_slugs: Vec<String> = m
        .leaves
        .iter()
        .filter(|l| {
            m.tree
                .last_compiled_at
                .as_ref()
                .is_none_or(|ts| &l.collected_at > ts)
        })
        .map(|l| l.slug.as_str().to_string())
        .collect();
    let manifest_hash = pending::manifest_hash(dir.path()).unwrap();

    let result = compile::run_compile_with_provider_started_at(
        &cfg,
        Default::default(),
        &provider,
        &model,
        &started_at,
        Vec::new(),
        &m,
        &new_slugs,
        &manifest_hash,
        &mut Vec::new(),
    );

    let compile_result = result.unwrap();
    assert_eq!(compile_result.status, "compiled");

    // Both branches should exist on disk
    let branches_dir = dir.path().join("branch");
    assert!(branches_dir.join("memory-model.md").exists());
    assert!(branches_dir.join("type-system.md").exists());

    // Manifest reflects both branches
    let m = bo::engine::manifest::read(&bo_dir.join("manifest.json")).unwrap();
    assert_eq!(m.branches.len(), 2);
    let slugs: Vec<&str> = m.branches.iter().map(|b| b.slug.as_str()).collect();
    assert!(slugs.contains(&"memory-model"));
    assert!(slugs.contains(&"type-system"));

    // Created-at preservation (migrated from dropped live test)
    let memory_model = m
        .branches
        .iter()
        .find(|b| b.slug.as_str() == "memory-model")
        .expect("memory-model branch present");
    assert_eq!(
        memory_model.created_at, ts_last_compile,
        "created_at must be preserved across an incremental update"
    );
}

#[test]
fn compile_all_leaves_deleted_repair_handles_missing_files() {
    // Create a tree with leaves and a branch, then delete all leaf files.
    // The stale-repair pass should prune orphan leaves and remove the branch.
    // The compile itself returns noop since no leaves remain.
    let dir = tempfile::TempDir::new().unwrap();
    let bo_dir = dir.path().join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    fs::create_dir_all(dir.path().join("branch")).unwrap();

    let ts = Timestamp::parse("2025-06-01T10:00:00Z").unwrap();

    // Only write one leaf file; record two in manifest.
    let title = Title::parse("Survivor").unwrap();
    let url = Url::parse("https://example.com/survivor").unwrap();
    let content =
        bo::domain::leaf::format_content(Some(&title), &url, &ts, "This leaf still exists.");
    fs::write(dir.path().join("survivor.md"), content).unwrap();

    // Write branch file that references both survivor and missing leaf
    fs::write(
        dir.path().join("branch/mixed-branch.md"),
        "---\ntitle: Mixed Branch\n---\n\n# Mixed Branch\n\nBody.\n",
    )
    .unwrap();

    bo::engine::manifest::write(
        &bo_dir.join("manifest.json"),
        &Manifest {
            tree: TreeMeta {
                name: "repair-fixture".to_string(),
                created_at: ts.clone(),
                last_compiled_at: Some(Timestamp::parse("2025-06-01T11:00:00Z").unwrap()),
            },
            leaves: vec![
                Leaf {
                    slug: bo::domain::Slug::parse("survivor").unwrap(),
                    file: "survivor.md".to_string(),
                    title: Some(Title::parse("Survivor").unwrap()),
                    url: Url::parse("https://example.com/survivor").unwrap(),
                    collected_at: ts.clone(),
                    summary: None,
                },
                Leaf {
                    slug: bo::domain::Slug::parse("deleted").unwrap(),
                    file: "deleted.md".to_string(),
                    title: Some(Title::parse("Deleted").unwrap()),
                    url: Url::parse("https://example.com/deleted").unwrap(),
                    collected_at: ts.clone(),
                    summary: None,
                },
            ],
            branches: vec![Branch {
                slug: bo::domain::Slug::parse("mixed-branch").unwrap(),
                file: "branch/mixed-branch.md".to_string(),
                title: Title::parse("Mixed Branch").unwrap(),
                created_at: ts.clone(),
                updated_at: ts.clone(),
                leaves: vec![
                    bo::domain::Slug::parse("survivor").unwrap(),
                    bo::domain::Slug::parse("deleted").unwrap(),
                ],
            }],
        },
    )
    .unwrap();

    // Verify the deleted leaf file is actually missing
    assert!(!dir.path().join("deleted.md").exists());

    let cfg = make_config(dir.path());
    let result = compile::run_compile_with_options(&cfg, Default::default()).result;
    assert!(
        result.is_ok(),
        "compile should succeed after repair: {:?}",
        result.err()
    );

    let compile_result = result.unwrap();
    // With only 1 surviving leaf, compile returns noop (single_leaf or empty_tree)
    assert_eq!(compile_result.status, "noop");

    // Manifest cleaned: deleted leaf removed, branch removed (only 1 leaf left)
    let m = bo::engine::manifest::read(&bo_dir.join("manifest.json")).unwrap();
    assert!(
        !m.leaves.iter().any(|l| l.slug.as_str() == "deleted"),
        "deleted leaf should be pruned from manifest"
    );
    // Branch is removed because it fell below 2 leaves
    let branch_slugs: Vec<&str> = m.branches.iter().map(|b| b.slug.as_str()).collect();
    assert!(
        !branch_slugs.contains(&"mixed-branch"),
        "branch should be removed after leaf deletion"
    );
    // Branch file deleted from disk
    assert!(!dir.path().join("branch/mixed-branch.md").exists());
}

// ── system-prompt assertion tests ─────────────────────────────────────────

/// Generate `count` leaf files and return manifest `Leaf` records.
fn create_leaf_docs(
    dir: &std::path::Path,
    count: usize,
    prefix: &str,
    ts: &Timestamp,
) -> Vec<Leaf> {
    let mut leaves = Vec::new();
    for i in 1..=count {
        let slug = format!("{}-{:02}", prefix, i);
        let file = format!("{}.md", slug);
        let title_str = format!("{} {}", prefix, i);
        let body = format!("Content of leaf {} {}.", prefix, i);
        let t = Title::parse(&title_str).unwrap();
        let u = Url::parse(&format!("https://example.com/{}", file)).unwrap();
        let content = bo::domain::leaf::format_content(Some(&t), &u, ts, &body);
        fs::write(dir.join(&file), content).unwrap();
        leaves.push(Leaf {
            slug: bo::domain::Slug::parse(&slug).unwrap(),
            file,
            title: Some(t),
            url: u,
            collected_at: ts.clone(),
            summary: None,
        });
    }
    leaves
}

#[test]
fn compile_one_shot_uses_compile_system_prompt() {
    // 4 leaves < 40 threshold → single-pass Full path.
    let dir = setup_fixture_collection();
    let cfg = make_config(dir.path());
    let model = compile_model();
    let started_at = Timestamp::now();

    let canned = serde_json::json!({
        "branches": [
            {
                "title": "Rust Memory Safety",
                "body": "# Rust Memory Safety\n\nContent.",
                "leaves": ["rust-ownership.md", "memory-safety.md"]
            },
            {
                "title": "Systems Design in Rust",
                "body": "# Systems Design in Rust\n\nContent.",
                "leaves": ["safe-concurrency.md", "zero-cost-abstractions.md"]
            }
        ]
    })
    .to_string();

    let provider = CapturingProvider::new(vec![canned]);
    let manifest = bo::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    let mhash = pending::manifest_hash(dir.path()).unwrap();

    let result = compile::run_compile_with_provider_started_at(
        &cfg,
        Default::default(),
        &provider,
        &model,
        &started_at,
        Vec::new(),
        &manifest,
        &[],
        &mhash,
        &mut Vec::new(),
    );

    assert!(result.is_ok(), "compile failed: {:?}", result.err());

    let prompts = provider.system_prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1, "expected exactly one LLM call");
    assert!(
        prompts[0].contains("knowledge compilation engine for a personal document collection"),
        "one-shot should send COMPILE system prompt, got: {}",
        prompts[0]
    );
}

#[test]
fn compile_full_two_stage_uses_cluster_system_prompt() {
    // 40 leaves ≥ 40 threshold → two-stage Full path.
    let dir = tempfile::TempDir::new().unwrap();
    let bo_dir = dir.path().join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();

    let ts = Timestamp::parse("2025-06-01T10:00:00Z").unwrap();
    let leaves = create_leaf_docs(dir.path(), 40, "leaf", &ts);

    bo::engine::manifest::write(
        &bo_dir.join("manifest.json"),
        &Manifest {
            tree: TreeMeta {
                name: "two-stage-full-fixture".to_string(),
                created_at: ts.clone(),
                last_compiled_at: None,
            },
            leaves: leaves.clone(),
            branches: Vec::new(),
        },
    )
    .unwrap();

    let cfg = make_config(dir.path());
    let model = compile_model();
    let started_at = Timestamp::now();

    // Stage 1: 2 clusters of 20 leaves each.
    let slugs_1_20: Vec<String> = (1..=20).map(|i| format!("leaf-{:02}", i)).collect();
    let slugs_21_40: Vec<String> = (21..=40).map(|i| format!("leaf-{:02}", i)).collect();

    let stage1 = serde_json::json!({
        "clusters": [
            {"title": "First Cluster", "leaf_slugs": slugs_1_20},
            {"title": "Second Cluster", "leaf_slugs": slugs_21_40},
        ]
    })
    .to_string();

    // Stage 2: one synthesize response per cluster.
    let stage2_a = serde_json::json!({
        "title": "First Cluster",
        "body": "# First Cluster\n\nSynthesized content."
    })
    .to_string();
    let stage2_b = serde_json::json!({
        "title": "Second Cluster",
        "body": "# Second Cluster\n\nSynthesized content."
    })
    .to_string();

    let provider = CapturingProvider::new(vec![stage1, stage2_a, stage2_b]);
    let manifest = bo::engine::manifest::read(&bo_dir.join("manifest.json")).unwrap();
    let mhash = pending::manifest_hash(dir.path()).unwrap();

    let result = compile::run_compile_with_provider_started_at(
        &cfg,
        Default::default(),
        &provider,
        &model,
        &started_at,
        Vec::new(),
        &manifest,
        &[],
        &mhash,
        &mut Vec::new(),
    );

    assert!(result.is_ok(), "compile failed: {:?}", result.err());

    let prompts = provider.system_prompts.lock().unwrap();
    assert!(
        prompts.len() >= 2,
        "expected at least 2 LLM calls (stage 1 + stage 2), got {}",
        prompts.len()
    );
    assert!(
        prompts[0].contains("You are clustering documents for a knowledge tree compilation"),
        "stage 1 should send CLUSTER system prompt, got: {}",
        prompts[0]
    );
    for (i, p) in prompts.iter().enumerate().skip(1) {
        assert!(
            p.contains("knowledge compilation engine"),
            "stage 2 call {} should send COMPILE system prompt, got: {}",
            i,
            p
        );
    }
}

#[test]
fn compile_incremental_two_stage_uses_incremental_cluster_system_prompt() {
    // Existing tree: 4 compiled leaves, 1 branch, last_compiled_at set.
    // Add 16 new leaves (collected after last_compile) → 16 ≥ 15 threshold.
    let dir = tempfile::TempDir::new().unwrap();
    let bo_dir = dir.path().join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    fs::create_dir_all(dir.path().join("branch")).unwrap();

    let ts_old = Timestamp::parse("2025-06-01T10:00:00Z").unwrap();
    let ts_last_compile = Timestamp::parse("2025-06-02T00:00:00Z").unwrap();
    let ts_new = Timestamp::parse("2025-06-03T10:00:00Z").unwrap();

    // Existing compiled leaves (4).
    let existing_leaves = create_leaf_docs(dir.path(), 4, "existing", &ts_old);
    // New leaves (16).
    let new_leaves = create_leaf_docs(dir.path(), 16, "new", &ts_new);

    // Existing branch file on disk.
    fs::write(
        dir.path().join("branch/existing-branch.md"),
        "---\ntitle: Existing Branch\ncreated_at: 2025-06-02T00:00:00Z\nupdated_at: 2025-06-02T00:00:00Z\nleaves:\n  - existing-01\n  - existing-02\n  - existing-03\n  - existing-04\n---\n\n# Existing Branch\n\nOriginal body.\n",
    )
    .unwrap();

    let all_leaves: Vec<Leaf> = existing_leaves
        .into_iter()
        .chain(new_leaves.clone())
        .collect();

    bo::engine::manifest::write(
        &bo_dir.join("manifest.json"),
        &Manifest {
            tree: TreeMeta {
                name: "incremental-two-stage-fixture".to_string(),
                created_at: ts_old.clone(),
                last_compiled_at: Some(ts_last_compile.clone()),
            },
            leaves: all_leaves,
            branches: vec![Branch {
                slug: bo::domain::Slug::parse("existing-branch").unwrap(),
                file: "branch/existing-branch.md".to_string(),
                title: Title::parse("Existing Branch").unwrap(),
                created_at: ts_last_compile.clone(),
                updated_at: ts_last_compile.clone(),
                leaves: vec![
                    bo::domain::Slug::parse("existing-01").unwrap(),
                    bo::domain::Slug::parse("existing-02").unwrap(),
                    bo::domain::Slug::parse("existing-03").unwrap(),
                    bo::domain::Slug::parse("existing-04").unwrap(),
                ],
            }],
        },
    )
    .unwrap();

    let cfg = make_config(dir.path());
    let model = compile_model();
    let started_at = Timestamp::now();

    // new leaf slugs: new-01 through new-16
    let new_slug_values: Vec<String> = (1..=16).map(|i| format!("new-{:02}", i)).collect();

    // Stage 1: assign first 8 new leaves to the existing branch, rest to a new cluster.
    let assign_slugs: Vec<String> = (1..=8).map(|i| format!("new-{:02}", i)).collect();
    let new_cluster_slugs: Vec<String> = (9..=16).map(|i| format!("new-{:02}", i)).collect();

    let stage1 = serde_json::json!({
        "assignments": [
            {"branch_slug": "existing-branch", "leaf_slugs": assign_slugs}
        ],
        "new_clusters": [
            {"title": "New Concept", "leaf_slugs": new_cluster_slugs}
        ]
    })
    .to_string();

    // Stage 2: update response for existing branch + synthesize for new cluster.
    let stage2_update = serde_json::json!({
        "title": "Existing Branch",
        "body": "# Existing Branch\n\nUpdated body."
    })
    .to_string();
    let stage2_new = serde_json::json!({
        "title": "New Concept",
        "body": "# New Concept\n\nSynthesized content."
    })
    .to_string();

    let provider = CapturingProvider::new(vec![stage1, stage2_update, stage2_new]);
    let manifest = bo::engine::manifest::read(&bo_dir.join("manifest.json")).unwrap();
    let mhash = pending::manifest_hash(dir.path()).unwrap();

    let result = compile::run_compile_with_provider_started_at(
        &cfg,
        Default::default(),
        &provider,
        &model,
        &started_at,
        Vec::new(),
        &manifest,
        &new_slug_values,
        &mhash,
        &mut Vec::new(),
    );

    assert!(result.is_ok(), "compile failed: {:?}", result.err());

    let prompts = provider.system_prompts.lock().unwrap();
    assert!(
        prompts.len() >= 2,
        "expected at least 2 LLM calls (stage 1 + stage 2), got {}",
        prompts.len()
    );
    assert!(
        prompts[0].contains("You are clustering new documents into an existing knowledge tree"),
        "stage 1 should send INCREMENTAL_CLUSTER system prompt, got: {}",
        prompts[0]
    );
    for (i, p) in prompts.iter().enumerate().skip(1) {
        assert!(
            p.contains("knowledge compilation engine"),
            "stage 2 call {} should send COMPILE system prompt, got: {}",
            i,
            p
        );
    }
}
