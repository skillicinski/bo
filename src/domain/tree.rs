// Tree — the top-level entity in bo's knowledge graph.
//
// A tree is what `bo seed` initialises: it is the root that holds all
// branches and leaves.  The hierarchy is:
//
//   Tree
//    ├── branches/   (Branch files written by `bo compile`)
//    └── *.md        (Leaf files written by `bo collect`)
//
// A tree has no dedicated on-disk file of its own — its metadata lives in
// `~/.bo/config.json` (via `Config`).  This module provides the domain type
// and derived path helpers; all persistence is delegated to `config.rs`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── TreeConfig ─────────────────────────────────────────────────────────────────

/// Serialised metadata for the active tree, stored under the `"tree"` key
/// in `config.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeConfig {
    pub output_dir: PathBuf,

    /// Human-readable name for the tree. Derived from the output directory
    /// basename at seed time, or supplied via `bo seed --name`.
    #[serde(default)]
    pub name: Option<String>,

    /// ISO 8601 UTC timestamp recorded when `bo seed` first ran.
    #[serde(default)]
    pub created_at: Option<String>,
}

// ── Tree ──────────────────────────────────────────────────────────────────────

/// The top-level entity in bo's knowledge graph.
///
/// Constructed from a [`TreeConfig`] via [`Tree::from_config`].
/// The `name` is always populated — derived from the directory basename
/// or "unnamed" as a final fallback.
#[derive(Debug, Clone)]
pub struct Tree {
    /// Human-readable name for this tree.
    pub name: String,

    /// ISO 8601 UTC timestamp of when `bo seed` first ran for this tree.
    ///
    /// `None` for trees seeded before this field was introduced.
    pub created_at: Option<String>,

    /// Absolute path to the tree root directory.
    pub output_dir: PathBuf,
}

impl Tree {
    /// Construct a `Tree` from a [`TreeConfig`].
    ///
    /// `name` is taken from `config.name` when present.  When absent (old
    /// config), it is derived from the basename of `config.output_dir`,
    /// falling back to `"unnamed"` if the path has no final component.
    pub fn from_config(config: &TreeConfig) -> Self {
        let name = config.name.clone().unwrap_or_else(|| {
            config
                .output_dir
                .file_name()
                .map(|n: &std::ffi::OsStr| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| "unnamed".to_string())
        });

        Tree {
            name,
            created_at: config.created_at.clone(),
            output_dir: config.output_dir.clone(),
        }
    }

    /// Path to the directory that holds branch files for this tree.
    ///
    /// Equivalent to `{output_dir}/branches`.
    pub fn branches_dir(&self) -> PathBuf {
        self.output_dir.join("branches")
    }

    /// Path to the tree-local infrastructure directory: `{output_dir}/.bo/`.
    ///
    /// Holds operational metadata (index, state, version) separate from
    /// content files.
    pub fn infra_dir(&self) -> PathBuf {
        self.output_dir.join(".bo")
    }

    /// Path to the manifest file: `{output_dir}/.bo/manifest.json`.
    pub fn manifest_path(&self) -> PathBuf {
        self.infra_dir().join("manifest.json")
    }
}

// ── Free path helpers ──────────────────────────────────────────────────────────

/// Manifest path from a bare tree directory.
pub fn manifest_path(tree_dir: &Path) -> PathBuf {
    tree_dir.join(".bo").join("manifest.json")
}

/// Infra directory from a bare tree directory.
pub fn infra_dir(tree_dir: &Path) -> PathBuf {
    tree_dir.join(".bo")
}

#[cfg(test)]
#[path = "../tests/domain_tree_tests.rs"]
mod tests;
