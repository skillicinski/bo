// Manifest fs I/O — reads and writes the manifest to disk.
//
// Pure types and serialisation live in domain::manifest. This module owns
// the filesystem operations so domain stays free of I/O.

use crate::domain::manifest::{Manifest, ManifestError};
use crate::domain::tree::{infra_dir, manifest_path, Tree, TreeRuntimeState};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

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

// ── internals ─────────────────────────────────────────────────────────────────

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));
    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
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

// ── tree runtime state ────────────────────────────────────────────────────────

pub fn runtime_state(tree_dir: &Path) -> Result<TreeRuntimeState, ManifestError> {
    match read(&manifest_path(tree_dir)) {
        Ok(manifest) => Ok(TreeRuntimeState::Initialized(manifest)),
        Err(ManifestError::TreeNotInitialized) if infra_dir(tree_dir).exists() => {
            Ok(TreeRuntimeState::MissingManifest)
        }
        Err(ManifestError::TreeNotInitialized) => Ok(TreeRuntimeState::FreshSeeded),
        Err(error) => Err(error),
    }
}

pub fn manifest_or_empty_if_fresh(tree: &Tree) -> Result<Manifest, ManifestError> {
    match runtime_state(tree.path())? {
        TreeRuntimeState::Initialized(manifest) => Ok(manifest),
        TreeRuntimeState::FreshSeeded => Ok(tree.empty_manifest()),
        TreeRuntimeState::MissingManifest => Err(ManifestError::TreeNotInitialized),
    }
}

#[cfg(test)]
#[path = "../tests/engine_manifest_tests.rs"]
mod tests;
