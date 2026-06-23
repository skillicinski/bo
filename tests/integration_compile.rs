// Integration tests for `bo compile`.
//
// Tests that require a live OpenAI API key are marked `#[ignore]` so they do
// not run in CI without credentials.  Run them explicitly with:
//
//   OPENAI_API_KEY=sk-... cargo test --test integration_compile -- --ignored

use bo::domain::{Timestamp, Title};
use std::fs;

use bo::cli::compile;
use bo::domain::manifest::{self, LeafRecord, Manifest, TreeMeta};
use bo::engine::config::SeededConfig;

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
        let title = Title::new(doc.title);
        let url = bo::domain::Url::parse(doc.url).unwrap();
        let ts = Timestamp::parse("2025-06-01T10:00:00Z").unwrap();
        let content = bo::domain::leaf::format_content(Some(&title), &url, &ts, doc.body, None);
        fs::write(dir.path().join(doc.file), content).unwrap();

        leaves.push(LeafRecord {
            slug: bo::domain::Slug::parse(doc.file.trim_end_matches(".md")).unwrap(),
            file: doc.file.to_string(),
            title: Title::new(doc.title),
            url: bo::domain::Url::parse(doc.url).unwrap(),
            collected_at: Timestamp::parse("2025-06-01T10:00:00Z").unwrap(),
            summary: None,
        });
    }

    manifest::write(
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
            model: Some("gpt-4o-mini".to_string()), // cheaper model for tests
            compile_model: None,
            tree: None,
        },
        bo::domain::tree::TreeConfig {
            path: output_dir.to_path_buf(),
            name: "test-tree".to_string(),
            created_at: bo::domain::Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        },
    )
}

// ── live API tests (require OPENAI_API_KEY) ───────────────────────────────────

#[test]
#[ignore = "requires OPENAI_API_KEY"]
fn compile_creates_branches_directory() {
    let dir = setup_fixture_collection();
    let cfg = make_config(dir.path());

    let result = compile::run_compile_with_options(&cfg, Default::default());
    assert!(result.is_ok(), "compile failed: {:?}", result.err());

    assert!(
        dir.path().join("branches").exists(),
        "branches/ directory was not created"
    );
}

#[test]
#[ignore = "requires OPENAI_API_KEY"]
fn compile_produces_at_least_one_branch_file() {
    let dir = setup_fixture_collection();
    let cfg = make_config(dir.path());

    compile::run_compile_with_options(&cfg, Default::default()).unwrap();

    let branches_dir = dir.path().join("branches");
    let branch_files: Vec<_> = fs::read_dir(&branches_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .collect();

    assert!(
        !branch_files.is_empty(),
        "no branch files were written to branches/"
    );

    // Validate the first branch file has correct frontmatter
    let first_path = branch_files[0].path();
    let content = fs::read_to_string(&first_path).unwrap();
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
}

#[test]
#[ignore = "requires OPENAI_API_KEY"]
fn compile_gives_every_leaf_a_branches_field() {
    let dir = setup_fixture_collection();
    let cfg = make_config(dir.path());

    compile::run_compile_with_options(&cfg, Default::default()).unwrap();

    for doc in FIXTURE_DOCS {
        let leaf_path = dir.path().join(doc.file);
        let content = fs::read_to_string(&leaf_path).unwrap();
        let (mapping, _) = bo::domain::frontmatter::parse(&content).unwrap();
        assert!(
            mapping.get("branches").is_some(),
            "leaf {} missing 'branches' field after compile",
            doc.file
        );
    }
}

#[test]
#[ignore = "requires OPENAI_API_KEY"]
fn compile_does_not_create_index_jsonl() {
    let dir = setup_fixture_collection();
    let cfg = make_config(dir.path());

    compile::run_compile_with_options(&cfg, Default::default()).unwrap();

    assert!(!dir.path().join(".bo/index.jsonl").exists());
}

#[test]
#[ignore = "requires OPENAI_API_KEY"]
fn compile_rerun_preserves_created_at() {
    let dir = setup_fixture_collection();
    let cfg = make_config(dir.path());

    // First compile
    compile::run_compile_with_options(&cfg, Default::default()).unwrap();

    let branches_dir = dir.path().join("branches");
    let first_branch = fs::read_dir(&branches_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .expect("no branch files after first compile")
        .path();

    let content1 = fs::read_to_string(&first_branch).unwrap();
    let (m1, _) = bo::domain::frontmatter::parse(&content1).unwrap();
    let created_at_1 = m1
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    // Brief sleep to ensure timestamp differs if updated_at changes
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Second compile
    compile::run_compile_with_options(&cfg, Default::default()).unwrap();

    // Find the same branch (by slug/filename)
    let content2 = fs::read_to_string(&first_branch).unwrap();
    let (m2, _) = bo::domain::frontmatter::parse(&content2).unwrap();
    let created_at_2 = m2
        .get("created_at")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    assert_eq!(
        created_at_1, created_at_2,
        "created_at changed on second compile run"
    );
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
    let mut m = manifest::read(&manifest_path).unwrap();
    m.tree.last_compiled_at = Some(Timestamp::parse("2099-01-01T00:00:00Z").unwrap());
    manifest::write(&manifest_path, &m).unwrap();

    // Record manifest hash before "crash"
    let manifest_hash = bo::engine::pending::manifest_hash(tree_dir).unwrap();

    // Write a staged .tmp file (simulating a branch about to be written)
    fs::create_dir_all(tree_dir.join("branches")).unwrap();
    let staged_content = b"# Fake Branch\n\nThis should be rolled back.\n";
    let staged_path = tree_dir.join("branches/fake-branch.md.tmp");
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
            path: "branches/fake-branch.md".to_string(),
            content_hash: bo::engine::pending::content_hash(staged_content),
        }],
        deletes: vec![],
    };
    let pending_path = bo_dir.join("pending.json");
    bo::engine::pending::write(&pending_path, &pending).unwrap();

    // Now run compile — should detect stale pending, rollback, then proceed normally
    let cfg = make_config(tree_dir);
    let result = compile::run_compile_with_options(&cfg, Default::default());

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
        result.is_ok(),
        "compile should succeed after recovery: {:?}",
        result.err()
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
    let mut m = manifest::read(&manifest_path).unwrap();
    m.tree.last_compiled_at = Some(Timestamp::parse("2099-01-01T00:00:00Z").unwrap());
    manifest::write(&manifest_path, &m).unwrap();

    // Write a staged .tmp file
    fs::create_dir_all(tree_dir.join("branches")).unwrap();
    let branch_content =
        b"---\ntitle: Recovered Branch\n---\n\n# Recovered Branch\n\nThis was recovered.\n";
    let staged_path = tree_dir.join("branches/recovered-branch.md.tmp");
    let final_path = tree_dir.join("branches/recovered-branch.md");
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
            path: "branches/recovered-branch.md".to_string(),
            content_hash: bo::engine::pending::content_hash(branch_content),
        }],
        deletes: vec![],
    };
    let pending_path = bo_dir.join("pending.json");
    bo::engine::pending::write(&pending_path, &pending).unwrap();

    // Run compile — should roll forward
    let cfg = make_config(tree_dir);
    let result = compile::run_compile_with_options(&cfg, Default::default());

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
    let mut m = manifest::read(&manifest_path).unwrap();
    m.tree.last_compiled_at = Some(Timestamp::parse("2099-01-01T00:00:00Z").unwrap());
    manifest::write(&manifest_path, &m).unwrap();

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
    let m = manifest::read(&bo_dir.join("manifest.json")).unwrap();
    assert!(m.leaf_by_slug_str("interrupted").is_none());
    assert!(!bo_dir.join("pending.json").exists());
}
