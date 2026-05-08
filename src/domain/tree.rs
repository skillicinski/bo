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
use std::path::PathBuf;

// ── TreeConfig ─────────────────────────────────────────────────────────────────

/// Serialised metadata for the active tree, stored under the `"tree"` key
/// in `config.json`.
#[derive(Debug, Serialize, Deserialize)]
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
/// Constructed from a [`TreeConfig`] via [`Tree::from_config`].  Fields that
/// were not present in older config files are represented as `None`.
#[derive(Debug, Clone)]
pub struct Tree {
    /// Human-readable name for this tree.
    ///
    /// `Some` when the config was written by a version of bo that records the
    /// name; `None` for trees seeded before this field was introduced.
    pub name: Option<String>,

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
        let name = config.name.clone().or_else(|| {
            config
                .output_dir
                .file_name()
                .map(|n: &std::ffi::OsStr| n.to_string_lossy().into_owned())
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
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn full_config() -> TreeConfig {
        TreeConfig {
            output_dir: PathBuf::from("/tmp/my-research"),
            name: Some("my-research".to_string()),
            created_at: Some("2026-04-14T09:00:00Z".to_string()),
        }
    }

    fn old_config() -> TreeConfig {
        TreeConfig {
            output_dir: PathBuf::from("/tmp/old-tree"),
            name: None,
            created_at: None,
        }
    }

    #[test]
    fn from_full_config_preserves_all_fields() {
        let tree = Tree::from_config(&full_config());
        assert_eq!(tree.name.as_deref(), Some("my-research"));
        assert_eq!(tree.created_at.as_deref(), Some("2026-04-14T09:00:00Z"));
        assert_eq!(tree.output_dir, PathBuf::from("/tmp/my-research"));
    }

    #[test]
    fn from_old_config_derives_name_from_dir_basename() {
        let tree = Tree::from_config(&old_config());
        // name is derived from "old-tree" (basename of /tmp/old-tree)
        assert_eq!(tree.name.as_deref(), Some("old-tree"));
        assert!(tree.created_at.is_none());
    }

    #[test]
    fn from_config_name_explicit_beats_derived() {
        // When config.name is set, it wins over the directory basename
        let config = TreeConfig {
            output_dir: PathBuf::from("/tmp/dir-name"),
            name: Some("explicit-name".to_string()),
            created_at: None,
        };
        let tree = Tree::from_config(&config);
        assert_eq!(tree.name.as_deref(), Some("explicit-name"));
    }

    #[test]
    fn branches_dir_is_output_dir_slash_branches() {
        let tree = Tree::from_config(&full_config());
        assert_eq!(
            tree.branches_dir(),
            PathBuf::from("/tmp/my-research/branches")
        );
    }
}
