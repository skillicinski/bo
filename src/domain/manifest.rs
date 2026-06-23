// Manifest — the single source of truth for a tree's topology.
//
// `{tree}/.bo/manifest.json` holds tree metadata, the leaf roster, and the
// branch roster. Cross-references are unidirectional: branches list their
// leaf slugs; the inverse (which branches contain a given leaf) is computed
// in-memory at call time. The manifest is the only tree-state store. Missing
// or corrupt manifests are surfaced as errors; there is no secondary
// reconstruction path.

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
    /// Look up a leaf by slug string (convenience for contexts that have a raw &str).
    pub fn leaf_by_slug_str(&self, slug: &str) -> Option<&LeafRecord> {
        self.leaves.iter().find(|l| l.slug.as_str() == slug)
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

    /// Inverse of cross-reference: which branches contain a given leaf.
    /// Computed in-memory at call time; the manifest does not persist this
    /// direction.
    pub fn branches_for_leaf(&self, leaf_slug: &Slug) -> Vec<&BranchRecord> {
        self.branches
            .iter()
            .filter(|b| b.leaves.iter().any(|s| s == leaf_slug))
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

/// Write an empty manifest at `{tree_dir}/.bo/manifest.json` if none exists.
/// Used by tests that stage manifest entries directly without running collect.
pub fn ensure_empty_manifest(tree_dir: &Path, name: &str) {
    let manifest_path = tree_dir.join(".bo").join("manifest.json");
    if manifest_path.exists() {
        return;
    }
    write(
        &manifest_path,
        &Manifest {
            tree: TreeMeta {
                name: name.to_string(),
                created_at: Timestamp::now(),
                last_compiled_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        },
    )
    .expect("failed to write empty manifest");
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
