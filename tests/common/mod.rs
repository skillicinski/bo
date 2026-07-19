#![allow(dead_code)]

//! Shared harness for `tests/` integration suites.
//!
//! Repeated environment construction only: launch the installed `bo` binary
//! against an isolated HOME, seed a named tree, bootstrap/read/write a
//! state, append typed leaf records, and the canonical fixture
//! collection shared by the synthesis suites.
//!
//! No assertions on command output, no fixture-builder hierarchy, and no
//! scenario data beyond the synthesis fixtures shared verbatim by two suites.
//! Each suite keeps its command arguments, expected output, and assertions at
//! the call site.
//!
//! Included by each integration test file via `mod common;`. Cargo does not
//! compile this as a standalone test target.

use bo::domain::state::{TreeMetadata, TreeState};
use bo::domain::tree::TreeConfig;
use bo::domain::{Leaf, Slug, Timestamp, Title, Url};
use bo::engine::config::{Config, SeededConfig};
use bo::engine::llm::Provider;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// Harness epoch for default timestamps. Scenarios that care about ordering
/// pass their own timestamp by constructing the record directly.
const EPOCH: &str = "2025-01-01T00:00:00Z";

// ── process + HOME ───────────────────────────────────────────────────────────

/// Launch the installed `bo` binary with `HOME` pointed at an isolated temp dir.
pub fn bo(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bo"));
    cmd.env("HOME", home);
    cmd
}

/// Seed a tree named `name` under `home` with the default provider/model
/// (openai / gpt-4.1-mini). Panics if seed exits non-zero. Returns the tree dir.
pub fn seed(home: &Path, name: &str) -> PathBuf {
    seed_with(home, name, "openai", "gpt-4.1-mini")
}

/// Seed a tree named `name` under `home` with an explicit provider/model.
pub fn seed_with(home: &Path, name: &str, provider: &str, model: &str) -> PathBuf {
    let dir = home.join(name);
    let out = bo(home)
        .args([
            "seed",
            "--path",
            dir.to_str().unwrap(),
            "--name",
            name,
            "--provider",
            provider,
            "--model",
            model,
        ])
        .output()
        .expect("failed to run bo seed");
    assert!(
        out.status.success(),
        "seed failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    dir
}

// ── state ─────────────────────────────────────────────────────────────────

/// Path to a tree's state.
fn state_path(tree: &Path) -> PathBuf {
    tree.join(".bo").join("state.json")
}

/// Ensure an empty state exists; no-op if one is already present.
pub fn ensure_state(tree: &Path) {
    if !state_path(tree).exists() {
        write_state(tree, &empty_state("tree"));
    }
}

/// Read the tree state. Panics if absent or invalid.
pub fn read_state(tree: &Path) -> TreeState {
    bo::engine::state::read(&state_path(tree)).unwrap()
}

/// Write the tree state, creating `.bo/` if needed.
pub fn write_state(tree: &Path, state: &TreeState) {
    let path = state_path(tree);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    bo::engine::state::write(&path, state).unwrap();
}

/// Append a leaf record to the state (does not touch files on disk).
pub fn append_leaf(tree: &Path, leaf: Leaf) {
    let mut m = read_state(tree);
    m.leaves.push(leaf);
    write_state(tree, &m);
}

/// An empty state named `name` stamped at the harness epoch.
fn empty_state(name: &str) -> TreeState {
    TreeState {
        tree: TreeMetadata {
            name: name.to_string(),
            created_at: ts(EPOCH),
            last_synthesized_at: None,
        },
        leaves: Vec::new(),
        branches: Vec::new(),
    }
}

fn ts(s: &str) -> Timestamp {
    Timestamp::parse(s).unwrap()
}

// ── synthesis-suite fixture collection ───────────────────────────────────────
//
// Shared verbatim by `integration_synthesize` and `integration_synthesize_dry_run`.

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

/// Build the canonical synthesis fixture tree in a fresh temp dir.
pub fn setup_fixture_collection() -> TempDir {
    let dir = TempDir::new().unwrap();
    let mut leaves = Vec::new();
    for doc in FIXTURE_DOCS {
        let title = Title::parse(doc.title).ok();
        let url = Url::parse(doc.url).unwrap();
        let collected = ts("2025-06-01T10:00:00Z");
        let content = bo::domain::leaf::format_content(title.as_ref(), &url, &collected, doc.body);
        fs::write(dir.path().join(doc.file), content).unwrap();
        leaves.push(Leaf {
            slug: Slug::parse(doc.file.trim_end_matches(".md")).unwrap(),
            file: doc.file.to_string(),
            title,
            url,
            collected_at: collected,
            summary: None,
        });
    }
    write_state(
        dir.path(),
        &TreeState {
            tree: TreeMetadata {
                name: "synthesis-fixture".to_string(),
                created_at: ts("2025-06-01T09:00:00Z"),
                last_synthesized_at: None,
            },
            leaves,
            branches: Vec::new(),
        },
    );
    dir
}

/// A `SeededConfig` over `dir` for the given provider/model (tree "test-tree").
pub fn seeded_config(dir: &Path, provider: Provider, model: &str) -> SeededConfig {
    SeededConfig::new(
        Config {
            provider,
            model: model.to_string(),
            synthesis_model: None,
            base_url: None,
            tree: None,
        },
        TreeConfig {
            path: dir.to_path_buf(),
            name: "test-tree".to_string(),
            created_at: ts("2026-01-01T00:00:00Z"),
        },
    )
}

// ── byte-exact tree snapshot ─────────────────────────────────────────────────
//
// Shared by the agent smoke and dry-run suites to assert a dry-run wrote no
// bytes: snapshot before and after, compare for equality.

/// Recursively collect every regular file under `dir` as `(relative path, raw
/// bytes)`, sorted by path. Byte-exact and deterministic across platforms.
pub fn snapshot_tree(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut entries = Vec::new();
    collect_files(dir, dir, &mut entries);
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read_dir.filter_map(Result::ok).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            out.push((rel, fs::read(&path).unwrap()));
        }
    }
}
