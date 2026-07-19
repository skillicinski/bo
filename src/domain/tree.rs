// Tree — the top-level entity in bo's knowledge graph.
//
// A tree is what `bo seed` initialises: it is the root that holds all
// branches and leaves. The hierarchy is:
//
//   Tree
//    ├── branch/   (Branch files written by `bo synthesize`)
//    ├── leaf/     (Leaf files written by `bo collect`)
//    └── .bo/      (state.json + pending.json runtime state)
//
// The active tree's metadata lives in `~/.bo/config.json` under `tree`.
// Runtime tree state lives in `{tree}/.bo/state.json` once collection starts.

use crate::domain::state::{TreeMetadata, TreeState};
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

    pub fn empty_state(&self) -> TreeState {
        TreeState {
            tree: TreeMetadata {
                name: self.name.clone(),
                created_at: self.created_at.clone(),
                last_synthesized_at: None,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeLoadState {
    FreshSeeded,
    Loaded(TreeState),
    MissingState,
}

// ── Free path helpers ──────────────────────────────────────────────────────────

/// Branch directory from a bare tree directory.
pub fn branch_dir(tree_dir: &Path) -> PathBuf {
    tree_dir.join("branch")
}

/// Leaf directory from a bare tree directory.
pub fn leaf_dir(tree_dir: &Path) -> PathBuf {
    tree_dir.join("leaf")
}

/// Tree state path from a bare tree directory.
pub fn state_path(tree_dir: &Path) -> PathBuf {
    infra_dir(tree_dir).join("state.json")
}

/// Infra directory from a bare tree directory.
pub fn infra_dir(tree_dir: &Path) -> PathBuf {
    tree_dir.join(".bo")
}

#[cfg(test)]
#[path = "../tests/domain_tree_tests.rs"]
mod tests;
