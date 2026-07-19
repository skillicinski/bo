// TreeState — the single source of truth for a tree's topology.
//
// `{tree}/.bo/state.json` holds tree metadata, the leaf roster, and the
// branch roster. Cross-references are unidirectional: branches list their
// leaf slugs; the inverse (which branches contain a given leaf) is computed
// in-memory at call time. The tree state is the only topology record. Missing
// or corrupt state files are surfaced as errors; there is no secondary
// reconstruction path.
//
// Pure types and resolution logic live here. Filesystem I/O lives in
// `engine::state` so domain stays free of I/O.

use crate::domain::{Branch, Leaf, Slug, Timestamp};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeState {
    pub tree: TreeMetadata,
    /// All collected leaves, in collection order.
    pub leaves: Vec<Leaf>,
    /// All synthesized branches, in synthesis order.
    pub branches: Vec<Branch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeMetadata {
    pub name: String,
    pub created_at: Timestamp,
    /// Set on successful synthesis. `None` until the first synthesis run.
    pub last_synthesized_at: Option<Timestamp>,
}

// ── errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum TreeStateError {
    Io(io::Error),
    Parse(serde_json::Error),
    /// `state.json` does not exist at the requested path.
    TreeNotInitialized,
}

impl fmt::Display for TreeStateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TreeStateError::Io(e) => write!(f, "tree state I/O error: {}", e),
            TreeStateError::Parse(e) => write!(f, "tree state parse error: {}", e),
            TreeStateError::TreeNotInitialized => {
                write!(f, "tree not initialized; run bo seed")
            }
        }
    }
}

impl From<io::Error> for TreeStateError {
    fn from(e: io::Error) -> Self {
        TreeStateError::Io(e)
    }
}

impl From<serde_json::Error> for TreeStateError {
    fn from(e: serde_json::Error) -> Self {
        TreeStateError::Parse(e)
    }
}

// ── resolution helpers ────────────────────────────────────────────────────────

impl TreeState {
    /// Look up a branch by slug string (convenience).
    pub fn branch_by_slug_str(&self, slug: &str) -> Option<&Branch> {
        self.branches.iter().find(|b| b.slug.as_str() == slug)
    }

    /// Leaves that have not been seen by a synthesis pass.
    ///
    /// A leaf is unsynthesized iff `tree.last_synthesized_at` is `None` or
    /// `leaf.collected_at > tree.last_synthesized_at`. Uses typed Ord comparison.
    pub fn unsynthesized_leaves(&self) -> Vec<&Leaf> {
        match &self.tree.last_synthesized_at {
            None => self.leaves.iter().collect(),
            Some(last) => self
                .leaves
                .iter()
                .filter(|l| &l.collected_at > last)
                .collect(),
        }
    }

    /// Inverse of cross-reference: which branches contain a given leaf.
    /// Computed in-memory at call time; the tree state does not persist this
    /// direction.
    pub fn branches_for_leaf(&self, leaf_slug: &Slug) -> Vec<&Branch> {
        self.branches
            .iter()
            .filter(|b| b.leaves.iter().any(|s| s == leaf_slug))
            .collect()
    }
}

#[cfg(test)]
#[path = "../tests/domain_state_tests.rs"]
mod tests;
