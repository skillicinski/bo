// TreeState fs I/O — reads and writes the tree state to disk.
//
// Pure types and serialisation live in domain::state. This module owns
// the filesystem operations so domain stays free of I/O.

use crate::domain::state::{TreeState, TreeStateError};
use crate::domain::tree::{infra_dir, state_path, Tree, TreeLoadState};
use std::collections::HashSet;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

// ── public I/O ────────────────────────────────────────────────────────────────

/// Read tree state from disk.
///
/// A sibling legacy `.bo/manifest.json` is rejected before the state file
/// is touched, whether `state.json` is present or absent: a tree carrying
/// both (or only the legacy file) is in a split-brain state and must be
/// migrated rather than silently read.
///
/// - Legacy manifest present (alone or alongside state) → `Err(LegacyManifestFound)`
/// - State present + valid JSON   → `Ok(TreeState)`
/// - State present + invalid JSON → `Err(Parse)`
/// - State absent                 → `Err(TreeNotInitialized)`
pub fn read(path: &Path) -> Result<TreeState, TreeStateError> {
    ensure_current_format(path)?;
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str(&s).map_err(TreeStateError::Parse),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(TreeStateError::TreeNotInitialized),
        Err(e) => Err(TreeStateError::Io(e)),
    }
}

/// Atomically write tree state to disk.
///
/// Writes to `{path}.tmp`, fsyncs the file, then renames into place. POSIX rename
/// guarantees atomic replacement on a single filesystem.
///
/// Refuses to write if a sibling legacy `.bo/manifest.json` exists: writing
/// `state.json` next to it would lock in a split-brain tree.
///
/// In debug builds, panics if leaf or branch slug uniqueness is violated.
/// Release builds skip the check (zero cost).
pub fn write(path: &Path, state: &TreeState) -> Result<(), TreeStateError> {
    ensure_current_format(path)?;
    debug_assert_unique_slugs(state);
    let json = serde_json::to_string_pretty(state)?;
    atomic_write(path, json.as_bytes())?;
    Ok(())
}

// ── internals ─────────────────────────────────────────────────────────────────

/// Reject legacy or split-brain tree metadata before any read, write, or
/// recovery mutates the tree.
pub(crate) fn ensure_current_format(path: &Path) -> Result<(), TreeStateError> {
    if path
        .parent()
        .is_some_and(|parent| parent.join("manifest.json").exists())
    {
        return Err(TreeStateError::LegacyManifestFound);
    }
    Ok(())
}

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

fn debug_assert_unique_slugs(s: &TreeState) {
    if !cfg!(debug_assertions) {
        return;
    }
    let mut leaf_slugs = HashSet::new();
    for l in &s.leaves {
        assert!(
            leaf_slugs.insert(l.slug.as_str()),
            "duplicate leaf slug: {}",
            l.slug
        );
    }
    let mut branch_slugs = HashSet::new();
    for b in &s.branches {
        assert!(
            branch_slugs.insert(b.slug.as_str()),
            "duplicate branch slug: {}",
            b.slug
        );
    }
}

// ── tree runtime state ────────────────────────────────────────────────────────

pub fn load_state(tree_dir: &Path) -> Result<TreeLoadState, TreeStateError> {
    match read(&state_path(tree_dir)) {
        Ok(state) => Ok(TreeLoadState::Loaded(state)),
        Err(TreeStateError::TreeNotInitialized) if infra_dir(tree_dir).exists() => {
            // .bo exists but state.json doesn't: existing tree content with no
            // state record. A legacy manifest.json is already caught by read().
            Ok(TreeLoadState::MissingState)
        }
        Err(TreeStateError::TreeNotInitialized) => Ok(TreeLoadState::FreshSeeded),
        // LegacyManifestFound (and any other read error) propagates unchanged.
        Err(error) => Err(error),
    }
}

pub fn state_or_empty_if_fresh(tree: &Tree) -> Result<TreeState, TreeStateError> {
    match load_state(tree.path())? {
        TreeLoadState::Loaded(state) => Ok(state),
        TreeLoadState::FreshSeeded => Ok(tree.empty_state()),
        TreeLoadState::MissingState => Err(TreeStateError::TreeNotInitialized),
    }
}

#[cfg(test)]
#[path = "../tests/engine_state_tests.rs"]
mod tests;
