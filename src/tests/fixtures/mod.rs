//! Shared test fixtures — tree setup, leaf/branch construction helpers.

use crate::domain::manifest::{Manifest, TreeMeta};
use crate::domain::Leaf;
use crate::domain::{Slug, Timestamp, Title, Url};
use crate::engine::config;
use std::fs;
use std::path::{Path, PathBuf};
use tempfile::TempDir;

// ─── test helpers ────────────────────────────────────────────────────────────

/// Construct a validated Title; panics on invalid input (test-only convenience).
pub fn title(s: &str) -> Title {
    Title::parse(s).expect("invalid test title")
}

/// Construct a validated Url; panics on invalid input (test-only convenience).
pub fn url(s: &str) -> Url {
    Url::parse(s).expect("invalid test URL")
}

// ─── tree setup ──────────────────────────────────────────────────────────────

/// Create a minimal seeded tree with an empty manifest inside `tmp`.
/// Returns `(tree_dir, config_path)`.
pub fn setup_tree(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let tree_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");
    fs::create_dir_all(&tree_dir).unwrap();
    let bo_dir = tree_dir.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    crate::engine::manifest::write(
        &bo_dir.join("manifest.json"),
        &Manifest {
            tree: TreeMeta {
                name: "tree".to_string(),
                created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        },
    )
    .unwrap();

    config::write_config(
        &config::Config {
            provider: crate::engine::llm::Provider::OpenAI,
            tree: Some(crate::domain::tree::TreeConfig {
                path: tree_dir.clone(),
                name: "tree".to_string(),
                created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            }),
            model: "gpt-4.1-mini".to_string(),
            compile_model: None,
            base_url: None,
        },
        &config_path,
    )
    .unwrap();

    (tree_dir, config_path)
}

pub fn auth_path_for_config(config_path: &Path) -> PathBuf {
    config_path.with_file_name("auth.json")
}

// ─── leaf helpers ────────────────────────────────────────────────────────────

/// Add a leaf file (`.md`) and a corresponding manifest entry.
pub fn add_leaf(tree_dir: &Path, file: &str) {
    add_manifest_leaf(tree_dir, file);
    let path = tree_dir.join(file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, "# content\n").unwrap();
}

/// Add a manifest entry for a leaf without creating the file on disk.
/// Uses an atomic counter to generate unique slugs.
pub fn add_manifest_leaf(tree_dir: &Path, file: &str) {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let manifest_path = tree_dir.join(".bo/manifest.json");
    let mut manifest = crate::engine::manifest::read(&manifest_path).unwrap();
    let idx = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let slug = Slug::parse(&format!("leaf-{}", idx)).unwrap();
    manifest.leaves.push(Leaf {
        slug,
        file: file.to_string(),
        title: Some(title(file.trim_end_matches(".md"))),
        url: url("https://example.com/test"),
        collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
        summary: None,
    });
    crate::engine::manifest::write(&manifest_path, &manifest).unwrap();
}

// ─── record constructors ─────────────────────────────────────────────────────

/// Construct a `Leaf` with sensible defaults for tests.
pub fn make_leaf_record(slug: &str, file: &str) -> Leaf {
    Leaf {
        slug: Slug::parse(slug).expect("invalid test slug"),
        file: file.to_string(),
        title: Some(title(file.trim_end_matches(".md"))),
        url: url(&format!("https://example.com/{slug}")),
        collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
        summary: None,
    }
}

/// Construct a `Manifest` with the given leaves and empty branches.
pub fn make_manifest(name: &str, leaves: Vec<Leaf>) -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: name.to_string(),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            last_compiled_at: None,
        },
        leaves,
        branches: Vec::new(),
    }
}
