#![allow(dead_code)]

//! Shared harness for `tests/` integration suites.
//!
//! Repeated environment construction only: launch the installed `bo` binary
//! against an isolated HOME, seed a named tree, bootstrap/read/write a
//! manifest, append typed leaf/branch records, and the canonical fixture
//! collection shared by the compile suites.
//!
//! No assertions on command output, no fixture-builder hierarchy, and no
//! scenario data beyond the compile fixtures shared verbatim by two suites.
//! Each suite keeps its command arguments, expected output, and assertions at
//! the call site.
//!
//! Included by each integration test file via `mod common;`. Cargo does not
//! compile this as a standalone test target.

use bo::domain::manifest::{Manifest, TreeMeta};
use bo::domain::tree::TreeConfig;
use bo::domain::{Branch, Leaf, Slug, Timestamp, Title, Url};
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

// ── manifest ─────────────────────────────────────────────────────────────────

/// Path to a tree's manifest.
pub fn manifest_path(tree: &Path) -> PathBuf {
    tree.join(".bo").join("manifest.json")
}

/// Ensure an empty manifest exists; no-op if one is already present.
pub fn ensure_manifest(tree: &Path) {
    if !manifest_path(tree).exists() {
        write_manifest(tree, &empty_manifest("tree"));
    }
}

/// Read the tree manifest. Panics if absent or invalid.
pub fn read_manifest(tree: &Path) -> Manifest {
    bo::engine::manifest::read(&manifest_path(tree)).unwrap()
}

/// Write the tree manifest, creating `.bo/` if needed.
pub fn write_manifest(tree: &Path, manifest: &Manifest) {
    let path = manifest_path(tree);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    bo::engine::manifest::write(&path, manifest).unwrap();
}

/// Append a leaf record to the manifest (does not touch files on disk).
pub fn append_leaf(tree: &Path, leaf: Leaf) {
    let mut m = read_manifest(tree);
    m.leaves.push(leaf);
    write_manifest(tree, &m);
}

/// Append a branch record to the manifest (does not touch files on disk).
pub fn append_branch(tree: &Path, branch: Branch) {
    let mut m = read_manifest(tree);
    m.branches.push(branch);
    write_manifest(tree, &m);
}

/// An empty manifest named `name` stamped at the harness epoch.
pub fn empty_manifest(name: &str) -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: name.to_string(),
            created_at: ts(EPOCH),
            last_compiled_at: None,
        },
        leaves: Vec::new(),
        branches: Vec::new(),
    }
}

fn ts(s: &str) -> Timestamp {
    Timestamp::parse(s).unwrap()
}

// ── record constructors with explicit inputs ────────────────────────────────

/// A leaf record with explicit title/url. Slug is derived from `file` (minus
/// `.md`); `collected_at` defaults to the harness epoch.
pub fn leaf(file: &str, title: &str, url: &str) -> Leaf {
    let stem = file.trim_end_matches(".md");
    Leaf {
        slug: Slug::parse(stem).unwrap_or_else(|_| Slug::generate(stem, url)),
        file: file.to_string(),
        title: Title::parse(title).ok(),
        url: Url::parse(url).unwrap(),
        collected_at: ts(EPOCH),
        summary: None,
    }
}

/// A branch record titled by its slug, filing under `branch/<slug>.md`.
pub fn branch(slug: &str, leaves: &[&str]) -> Branch {
    Branch {
        slug: Slug::parse(slug).unwrap(),
        file: format!("branch/{slug}.md"),
        title: Title::parse(slug).unwrap(),
        created_at: ts(EPOCH),
        updated_at: ts(EPOCH),
        leaves: leaves.iter().map(|s| Slug::parse(s).unwrap()).collect(),
    }
}

// ── compile-suite fixture collection ─────────────────────────────────────────
//
// Shared verbatim by `integration_compile` and `integration_compile_dry_run`.

pub struct FixtureDoc {
    pub file: &'static str,
    pub title: &'static str,
    pub url: &'static str,
    pub body: &'static str,
}

pub const FIXTURE_DOCS: &[FixtureDoc] = &[
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

/// Build the canonical compile fixture tree in a fresh temp dir.
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
    write_manifest(
        dir.path(),
        &Manifest {
            tree: TreeMeta {
                name: "compile-fixture".to_string(),
                created_at: ts("2025-06-01T09:00:00Z"),
                last_compiled_at: None,
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
            compile_model: None,
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
