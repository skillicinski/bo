// Manifest — the single source of truth for a tree's topology.
//
// `{tree}/.bo/manifest.json` holds tree metadata, the leaf roster, and the
// branch roster. Cross-references are unidirectional: branches list their
// leaf slugs; the inverse (which branches contain a given leaf) is computed
// in-memory at call time. The manifest is the only tree-state store. Missing
// or corrupt manifests are surfaced as errors; there is no secondary
// reconstruction path.
//
// Pure types and resolution logic live here. Filesystem I/O lives in
// `engine::manifest` so domain stays free of I/O.

use crate::domain::{Branch, Leaf, Slug, Timestamp};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub tree: TreeMeta,
    /// All collected leaves, in collection order.
    pub leaves: Vec<Leaf>,
    /// All compiled branches, in compile order.
    pub branches: Vec<Branch>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeMeta {
    pub name: String,
    pub created_at: Timestamp,
    /// Set on successful compile. `None` until the first compile run.
    pub last_compiled_at: Option<Timestamp>,
}

// ── errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ManifestError {
    Io(io::Error),
    Parse(serde_json::Error),
    /// `manifest.json` does not exist at the requested path.
    TreeNotInitialized,
}

impl fmt::Display for ManifestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ManifestError::Io(e) => write!(f, "manifest I/O error: {}", e),
            ManifestError::Parse(e) => write!(f, "manifest parse error: {}", e),
            ManifestError::TreeNotInitialized => {
                write!(f, "tree not initialized; run bo seed")
            }
        }
    }
}

impl From<io::Error> for ManifestError {
    fn from(e: io::Error) -> Self {
        ManifestError::Io(e)
    }
}

impl From<serde_json::Error> for ManifestError {
    fn from(e: serde_json::Error) -> Self {
        ManifestError::Parse(e)
    }
}

// ── resolution helpers ────────────────────────────────────────────────────────

impl Manifest {
    /// Look up a branch by slug string (convenience).
    pub fn branch_by_slug_str(&self, slug: &str) -> Option<&Branch> {
        self.branches.iter().find(|b| b.slug.as_str() == slug)
    }

    /// Leaves that have not been seen by a compile pass.
    ///
    /// A leaf is uncompiled iff `tree.last_compiled_at` is `None` or
    /// `leaf.collected_at > tree.last_compiled_at`. Uses typed Ord comparison.
    pub fn uncompiled_leaves(&self) -> Vec<&Leaf> {
        match &self.tree.last_compiled_at {
            None => self.leaves.iter().collect(),
            Some(last) => self
                .leaves
                .iter()
                .filter(|l| &l.collected_at > last)
                .collect(),
        }
    }

    /// Inverse of cross-reference: which branches contain a given leaf.
    /// Computed in-memory at call time; the manifest does not persist this
    /// direction.
    pub fn branches_for_leaf(&self, leaf_slug: &Slug) -> Vec<&Branch> {
        self.branches
            .iter()
            .filter(|b| b.leaves.iter().any(|s| s == leaf_slug))
            .collect()
    }
}

#[cfg(test)]
#[path = "../tests/domain_manifest_tests.rs"]
mod tests;
