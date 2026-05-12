// Integration tests for `bo compile`.
//
// Tests that require a live OpenAI API key are marked `#[ignore]` so they do
// not run in CI without credentials.  Run them explicitly with:
//
//   OPENAI_API_KEY=sk-... cargo test --test integration_compile -- --ignored

use std::fs;

use bo::cli::compile;
use bo::domain::index;
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
    let index_path = dir.path().join("index.jsonl");

    for doc in FIXTURE_DOCS {
        bo::domain::leaf::write(
            &dir.path().join(doc.file),
            Some(doc.title),
            doc.url,
            "2025-06-01T10:00:00Z",
            doc.body,
            None,
        )
        .unwrap();

        index::append_entry(
            &index_path,
            &index::IndexEntry {
                file: doc.file.to_string(),
                title: doc.title.to_string(),
                url: doc.url.to_string(),
            },
        )
        .unwrap();
    }

    dir
}

fn make_config(output_dir: &std::path::Path) -> SeededConfig {
    SeededConfig {
        tree: bo::domain::tree::TreeConfig {
            output_dir: output_dir.to_path_buf(),
            name: None,
            created_at: None,
        },
        model: Some("gpt-4o-mini".to_string()), // cheaper model for tests
    }
}

// ── live API tests (require OPENAI_API_KEY) ───────────────────────────────────

#[test]
#[ignore = "requires OPENAI_API_KEY"]
fn compile_creates_branches_directory() {
    let dir = setup_fixture_collection();
    let cfg = make_config(dir.path());

    let result = compile::cmd_compile(&cfg);
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

    compile::cmd_compile(&cfg).unwrap();

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
        mapping
            .get("compiled_at")
            .and_then(|v| v.as_str())
            .is_some(),
        "branch missing 'compiled_at' in frontmatter"
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

    compile::cmd_compile(&cfg).unwrap();

    let index_path = dir.path().join("index.jsonl");
    let entries = index::read_index(&index_path).unwrap();

    for entry in &entries {
        let leaf_path = dir.path().join(&entry.file);
        let content = fs::read_to_string(&leaf_path).unwrap();
        let (mapping, _) = bo::domain::frontmatter::parse(&content).unwrap();
        assert!(
            mapping.get("branches").is_some(),
            "leaf {} missing 'branches' field after compile",
            entry.file
        );
    }
}

#[test]
#[ignore = "requires OPENAI_API_KEY"]
fn compile_does_not_modify_index_jsonl() {
    let dir = setup_fixture_collection();
    let cfg = make_config(dir.path());

    let index_before = fs::read_to_string(dir.path().join("index.jsonl")).unwrap();

    compile::cmd_compile(&cfg).unwrap();

    let index_after = fs::read_to_string(dir.path().join("index.jsonl")).unwrap();
    assert_eq!(
        index_before, index_after,
        "index.jsonl was modified by bo compile"
    );
}

#[test]
#[ignore = "requires OPENAI_API_KEY"]
fn compile_rerun_preserves_compiled_at() {
    let dir = setup_fixture_collection();
    let cfg = make_config(dir.path());

    // First compile
    compile::cmd_compile(&cfg).unwrap();

    let branches_dir = dir.path().join("branches");
    let first_branch = fs::read_dir(&branches_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .expect("no branch files after first compile")
        .path();

    let content1 = fs::read_to_string(&first_branch).unwrap();
    let (m1, _) = bo::domain::frontmatter::parse(&content1).unwrap();
    let compiled_at_1 = m1
        .get("compiled_at")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    // Brief sleep to ensure timestamp differs if updated_at changes
    std::thread::sleep(std::time::Duration::from_secs(1));

    // Second compile
    compile::cmd_compile(&cfg).unwrap();

    // Find the same branch (by slug/filename)
    let content2 = fs::read_to_string(&first_branch).unwrap();
    let (m2, _) = bo::domain::frontmatter::parse(&content2).unwrap();
    let compiled_at_2 = m2
        .get("compiled_at")
        .and_then(|v| v.as_str())
        .unwrap()
        .to_string();

    assert_eq!(
        compiled_at_1, compiled_at_2,
        "compiled_at changed on second compile run"
    );
}
