// Manifest — the single source of truth for a tree's topology.
//
// `{tree}/.bo/manifest.json` holds tree metadata, the leaf roster, and the
// branch roster. Cross-references are unidirectional: branches list their
// leaf slugs; the inverse (which branches contain a given leaf) is computed
// in-memory at call time.
//
// As of 3b, `manifest.json` is the only tree-state store. Missing or corrupt
// manifests are surfaced as errors; there is no secondary reconstruction path.

use crate::domain::tree::Tree;
use crate::domain::{Slug, Timestamp, Title, Url};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Manifest {
    pub tree: TreeMeta,
    /// All collected leaves, in collection order.
    pub leaves: Vec<LeafRecord>,
    /// All compiled branches, in compile order.
    pub branches: Vec<BranchRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TreeMeta {
    pub name: String,
    pub created_at: Timestamp,
    /// Set on successful compile. `None` until the first compile run.
    pub last_compiled_at: Option<Timestamp>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeafRecord {
    pub slug: Slug,
    pub file: String,
    pub title: Title,
    pub url: Url,
    pub collected_at: Timestamp,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchRecord {
    pub slug: Slug,
    pub file: String,
    pub title: Title,
    /// First compile run that produced this branch. Preserved across recompiles.
    pub created_at: Timestamp,
    /// Most recent compile run that touched this branch. Updated every recompile.
    pub updated_at: Timestamp,
    /// True when one or more leaves referenced by this branch no longer exist
    /// in `manifest.leaves`. Always `false` in 3a; reserved for incremental
    /// compile (item 4).
    #[serde(default)]
    pub stale: bool,
    /// Slugs of leaves assigned to this branch. Canonical direction of the
    /// cross-reference.
    pub leaves: Vec<Slug>,
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
    /// Look up a leaf by slug. `None` if no such leaf exists.
    pub fn leaf_by_slug(&self, slug: &Slug) -> Option<&LeafRecord> {
        self.leaves.iter().find(|l| &l.slug == slug)
    }

    /// Look up a leaf by slug string (convenience for contexts that have a raw &str).
    pub fn leaf_by_slug_str(&self, slug: &str) -> Option<&LeafRecord> {
        self.leaves.iter().find(|l| l.slug.as_str() == slug)
    }

    /// Look up a branch by slug. `None` if no such branch exists.
    pub fn branch_by_slug(&self, slug: &Slug) -> Option<&BranchRecord> {
        self.branches.iter().find(|b| &b.slug == slug)
    }

    /// Look up a branch by slug string (convenience).
    pub fn branch_by_slug_str(&self, slug: &str) -> Option<&BranchRecord> {
        self.branches.iter().find(|b| b.slug.as_str() == slug)
    }

    /// Leaves that have not been seen by a compile pass.
    ///
    /// A leaf is uncompiled iff `tree.last_compiled_at` is `None` or
    /// `leaf.collected_at > tree.last_compiled_at`. Uses typed Ord comparison.
    pub fn uncompiled_leaves(&self) -> Vec<&LeafRecord> {
        match &self.tree.last_compiled_at {
            None => self.leaves.iter().collect(),
            Some(last) => self
                .leaves
                .iter()
                .filter(|l| &l.collected_at > last)
                .collect(),
        }
    }

    /// Branches whose `stale` flag is set. Always empty in 3a; populated by
    /// incremental compile (item 4) when a referenced leaf has been removed.
    pub fn stale_branches(&self) -> Vec<&BranchRecord> {
        self.branches.iter().filter(|b| b.stale).collect()
    }

    /// Resolve a branch's leaf-slug list to full `LeafRecord`s. Empty when
    /// the branch is unknown or owns no leaves.
    pub fn leaves_for_branch(&self, branch_slug: &Slug) -> Vec<&LeafRecord> {
        let Some(branch) = self.branch_by_slug(branch_slug) else {
            return Vec::new();
        };
        branch
            .leaves
            .iter()
            .filter_map(|s| self.leaf_by_slug(s))
            .collect()
    }

    /// Convenience: leaves_for_branch by slug string.
    pub fn leaves_for_branch_str(&self, branch_slug: &str) -> Vec<&LeafRecord> {
        let Some(branch) = self.branch_by_slug_str(branch_slug) else {
            return Vec::new();
        };
        branch
            .leaves
            .iter()
            .filter_map(|s| self.leaf_by_slug(s))
            .collect()
    }

    /// Inverse of `leaves_for_branch`: which branches contain a given leaf.
    /// Computed in-memory at call time; the manifest does not persist this
    /// direction of the cross-reference.
    pub fn branches_for_leaf(&self, leaf_slug: &Slug) -> Vec<&BranchRecord> {
        self.branches
            .iter()
            .filter(|b| b.leaves.iter().any(|s| s == leaf_slug))
            .collect()
    }

    /// Convenience: branches_for_leaf by slug string.
    pub fn branches_for_leaf_str(&self, leaf_slug: &str) -> Vec<&BranchRecord> {
        self.branches
            .iter()
            .filter(|b| b.leaves.iter().any(|s| s.as_str() == leaf_slug))
            .collect()
    }
}

// ── public I/O ────────────────────────────────────────────────────────────────

/// Read a manifest from disk.
///
/// - File present + valid JSON  → `Ok(Manifest)`
/// - File present + invalid JSON → `Err(Parse)`
/// - File absent                → `Err(TreeNotInitialized)`
pub fn read(path: &Path) -> Result<Manifest, ManifestError> {
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(ManifestError::Parse),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(ManifestError::TreeNotInitialized),
        Err(e) => Err(ManifestError::Io(e)),
    }
}

/// Atomically write a manifest to disk.
///
/// Writes to `{path}.tmp`, fsyncs the file, then renames into place. POSIX rename
/// guarantees atomic replacement on a single filesystem.
///
/// In debug builds, panics if leaf or branch slug uniqueness is violated.
/// Release builds skip the check (zero cost).
pub fn write(path: &Path, manifest: &Manifest) -> Result<(), ManifestError> {
    debug_assert_unique_slugs(manifest);
    let json = serde_json::to_string_pretty(manifest)?;
    atomic_write(path, json.as_bytes())?;
    Ok(())
}

/// Convenience: read from a `Tree`'s manifest path.
pub fn read_or_reconstruct(tree: &Tree) -> Result<Manifest, ManifestError> {
    read(&tree.manifest_path())
}

// ── internals ─────────────────────────────────────────────────────────────────

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = tmp_path_for(path);
    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn tmp_path_for(path: &Path) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(".tmp");
    PathBuf::from(s)
}

fn debug_assert_unique_slugs(m: &Manifest) {
    if !cfg!(debug_assertions) {
        return;
    }
    let mut leaf_slugs = HashSet::new();
    for l in &m.leaves {
        assert!(
            leaf_slugs.insert(l.slug.as_str()),
            "duplicate leaf slug: {}",
            l.slug
        );
    }
    let mut branch_slugs = HashSet::new();
    for b in &m.branches {
        assert!(
            branch_slugs.insert(b.slug.as_str()),
            "duplicate branch slug: {}",
            b.slug
        );
    }
}

#[cfg(test)]
#[path = "../tests/domain_manifest_tests.rs"]
mod tests;
