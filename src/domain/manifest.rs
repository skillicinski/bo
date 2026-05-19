// Manifest — the single source of truth for a tree's topology.
//
// `{tree}/.bo/manifest.json` holds tree metadata, the leaf roster, and the
// branch roster. Cross-references are unidirectional: branches list their
// leaf slugs; the inverse (which branches contain a given leaf) is computed
// in-memory at call time.
//
// In 3a the manifest coexists with the secondary store (`index.jsonl`,
// `state.json`, branch frontmatter). Reads come from the manifest;
// mutations write the manifest first as the commit point and then mirror
// to the secondary store. 3b removes the secondary store.
//
// Crash-safety beyond filesystem rename atomicity is 3b's domain.

use crate::domain::frontmatter;
use crate::domain::index;
use crate::domain::leaf;
use crate::domain::tree::{self, Tree};
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
    pub created_at: String,
    /// Set on successful compile. `None` until the first compile run.
    pub last_compiled_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LeafRecord {
    pub slug: String,
    pub file: String,
    pub title: String,
    pub url: String,
    pub collected_at: String,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchRecord {
    pub slug: String,
    pub file: String,
    pub title: String,
    /// First compile run that produced this branch. Preserved across recompiles.
    pub created_at: String,
    /// Most recent compile run that touched this branch. Updated every recompile.
    pub updated_at: String,
    /// True when one or more leaves referenced by this branch no longer exist
    /// in `manifest.leaves`. Always `false` in 3a; reserved for incremental
    /// compile (item 4).
    #[serde(default)]
    pub stale: bool,
    /// Slugs of leaves assigned to this branch. Canonical direction of the
    /// cross-reference.
    pub leaves: Vec<String>,
}

// ── errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum ManifestError {
    Io(io::Error),
    Parse(serde_json::Error),
    /// `manifest.json` does not exist at the requested path. After T2.1 ships,
    /// this is only returned when the secondary store is also absent.
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
    pub fn leaf_by_slug(&self, slug: &str) -> Option<&LeafRecord> {
        self.leaves.iter().find(|l| l.slug == slug)
    }

    /// Look up a branch by slug. `None` if no such branch exists.
    pub fn branch_by_slug(&self, slug: &str) -> Option<&BranchRecord> {
        self.branches.iter().find(|b| b.slug == slug)
    }

    /// Leaves that have not been seen by a compile pass.
    ///
    /// A leaf is uncompiled iff `tree.last_compiled_at` is `None` or
    /// `leaf.collected_at > tree.last_compiled_at`. RFC 3339 timestamps
    /// in UTC compare correctly under lexicographic ordering.
    pub fn uncompiled_leaves(&self) -> Vec<&LeafRecord> {
        match self.tree.last_compiled_at.as_deref() {
            None => self.leaves.iter().collect(),
            Some(last) => self
                .leaves
                .iter()
                .filter(|l| l.collected_at.as_str() > last)
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
    pub fn leaves_for_branch(&self, branch_slug: &str) -> Vec<&LeafRecord> {
        let Some(branch) = self.branch_by_slug(branch_slug) else {
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
    pub fn branches_for_leaf(&self, leaf_slug: &str) -> Vec<&BranchRecord> {
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
/// - File present + invalid JSON → `Err(Parse)` (does **not** trigger reconstruction;
///   auto-rebuilding over a corrupt file would silently destroy whatever the user broke).
/// - File absent                → `Err(TreeNotInitialized)` for now; T2.1 inserts a
///   reconstruction branch from the secondary store.
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

/// Read the manifest with reconstruction fallback.
///
/// If `manifest.json` is present, behaves like `read`. If absent, attempts to
/// rebuild the manifest from the secondary store (`index.jsonl`, leaf
/// frontmatter, `branches/`) and persists the result. Emits a one-line warning
/// to stderr when reconstruction succeeds.
///
/// Parse errors are surfaced unchanged — reconstruction never overwrites a
/// corrupt manifest. The user must `rm manifest.json` to opt into recovery.
///
/// Removed in 3b: the secondary store is gone, so missing-manifest becomes
/// unrecoverable.
pub fn read_or_reconstruct(tree: &Tree) -> Result<Manifest, ManifestError> {
    let mut stderr = io::stderr();
    read_or_reconstruct_into(tree, &mut stderr)
}

/// Internal entry point used by [`read_or_reconstruct`] and tests. Allows the
/// recovery warning destination to be injected.
pub(crate) fn read_or_reconstruct_into<W: Write>(
    tree: &Tree,
    warner: &mut W,
) -> Result<Manifest, ManifestError> {
    match read(&tree.manifest_path()) {
        Ok(m) => Ok(m),
        Err(ManifestError::TreeNotInitialized) => {
            let m = reconstruct_from_secondary(tree, warner)?;
            write(&tree.manifest_path(), &m)?;
            let _ = writeln!(
                warner,
                "manifest missing; reconstructed from secondary store"
            );
            Ok(m)
        }
        Err(e) => Err(e),
    }
}

// ── internals ─────────────────────────────────────────────────────────────────

// ── internals ───────────────────────────────────────────────────────────────────

// Removed in 3b: secondary store is gone, manifest becomes unrecoverable.
fn reconstruct_from_secondary<W: Write>(
    tree: &Tree,
    warner: &mut W,
) -> Result<Manifest, ManifestError> {
    let leaves = reconstruct_leaves(&tree.output_dir)?;
    let branches = reconstruct_branches(&tree.branches_dir())?;

    if leaves.is_empty() && branches.is_empty() {
        return Err(ManifestError::TreeNotInitialized);
    }

    let last_compiled_at = branches.iter().map(|b| b.updated_at.clone()).max();

    let name = tree.name.clone().unwrap_or_else(|| {
        tree.output_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unnamed".to_string())
    });
    let created_at = tree.created_at.clone().unwrap_or_else(|| {
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let _ = writeln!(
            warner,
            "tree config missing created_at; using current time {now}"
        );
        now
    });

    Ok(Manifest {
        tree: TreeMeta {
            name,
            created_at,
            last_compiled_at,
        },
        leaves,
        branches,
    })
}

fn reconstruct_leaves(tree_dir: &Path) -> Result<Vec<LeafRecord>, ManifestError> {
    let index_path = tree::index_path(tree_dir);
    if !index_path.exists() {
        return Ok(Vec::new());
    }
    let entries = index::read_index(&index_path)?;
    let mut out = Vec::with_capacity(entries.len());
    for entry in entries {
        let leaf_path = tree_dir.join(&entry.file);
        let mapping = leaf::read_frontmatter(&leaf_path).map_err(|e| {
            ManifestError::Io(io::Error::other(format!(
                "reconstruct leaf {}: {e}",
                entry.file
            )))
        })?;
        let slug = entry
            .file
            .strip_suffix(".md")
            .unwrap_or(&entry.file)
            .to_string();
        let title = mapping
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| entry.title.clone());
        let url = mapping
            .get("url")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| entry.url.clone());
        let collected_at = mapping
            .get("collected_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let summary = mapping
            .get("summary")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        out.push(LeafRecord {
            slug,
            file: entry.file,
            title,
            url,
            collected_at,
            summary,
        });
    }
    Ok(out)
}

fn reconstruct_branches(branches_dir: &Path) -> Result<Vec<BranchRecord>, ManifestError> {
    if !branches_dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(branches_dir)?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let content = fs::read_to_string(&path)?;
        let (mapping, _) = frontmatter::parse(&content).map_err(|e| {
            ManifestError::Io(io::Error::other(format!(
                "reconstruct branch {}: {e}",
                path.display()
            )))
        })?;
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let slug = filename
            .strip_suffix(".md")
            .unwrap_or(&filename)
            .to_string();
        let title = mapping
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let created_at = mapping
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let updated_at = mapping
            .get("updated_at")
            .and_then(|v| v.as_str())
            .unwrap_or(&created_at)
            .to_string();
        let leaves: Vec<String> = mapping
            .get("leaves")
            .and_then(|v| v.as_sequence())
            .map(|seq| {
                seq.iter()
                    .filter_map(|v| v.as_str())
                    .map(|s| s.strip_suffix(".md").unwrap_or(s).to_string())
                    .collect()
            })
            .unwrap_or_default();
        out.push(BranchRecord {
            slug,
            file: format!("branches/{filename}"),
            title,
            created_at,
            updated_at,
            stale: false,
            leaves,
        });
    }
    // Deterministic order — fs::read_dir is OS-dependent.
    out.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(out)
}

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
