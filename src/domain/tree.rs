// Tree — the top-level entity in bo's knowledge graph.
//
// A tree is what `bo seed` initialises: it is the root that holds all
// branches and leaves. The hierarchy is:
//
//   Tree
//    ├── branches/   (Branch files written by `bo compile`)
//    └── *.md        (Leaf files written by `bo collect`)
//
// The active tree's metadata lives in `~/.bo/config.json` under `tree`.
// Runtime tree state lives in `{tree}/.bo/manifest.json` once collection starts.

use crate::domain::manifest::{Manifest, TreeMeta};
use crate::domain::Timestamp;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── TreeConfig ─────────────────────────────────────────────────────────────────

/// Serialised metadata for the active tree, stored under the `"tree"` key
/// in `config.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeConfig {
    pub path: PathBuf,
    pub name: String,
    pub created_at: Timestamp,
}

// ── Tree ──────────────────────────────────────────────────────────────────────

/// The top-level entity in bo's knowledge graph.
#[derive(Debug, Clone)]
pub struct Tree {
    pub name: String,
    pub created_at: Timestamp,
    pub path: PathBuf,
}

impl Tree {
    pub fn from_config(config: &TreeConfig) -> Self {
        Tree {
            name: config.name.clone(),
            created_at: config.created_at.clone(),
            path: config.path.clone(),
        }
    }

    pub fn empty_manifest(&self) -> Manifest {
        Manifest {
            tree: TreeMeta {
                name: self.name.clone(),
                created_at: self.created_at.clone(),
                last_compiled_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.path.join(path)
    }

    /// Path to the directory that holds branch files for this tree.
    pub fn branches_dir(&self) -> PathBuf {
        self.path.join("branches")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeRuntimeState {
    FreshSeeded,
    Initialized(Manifest),
    MissingManifest,
}

// ── Free path helpers ──────────────────────────────────────────────────────────

/// Branches directory from a bare tree directory.
pub fn branches_dir(tree_dir: &Path) -> PathBuf {
    tree_dir.join("branches")
}

/// Manifest path from a bare tree directory.
pub fn manifest_path(tree_dir: &Path) -> PathBuf {
    infra_dir(tree_dir).join("manifest.json")
}

/// Infra directory from a bare tree directory.
pub fn infra_dir(tree_dir: &Path) -> PathBuf {
    tree_dir.join(".bo")
}

#[cfg(test)]
#[path = "../tests/domain_tree_tests.rs"]
mod tests;
