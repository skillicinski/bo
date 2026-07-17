// Commit-stage tests: dedup/slug/manifest/pending helpers exercised directly,
// without driving the full pipeline. Stage-unit tests for input/compute/journal
// live in their own files.

use super::*;
use crate::domain::manifest::{Manifest, TreeMeta};
use crate::domain::{Slug, Timestamp, Title, Url};
use std::fs;
use tempfile::TempDir;

#[test]
fn dedup_uses_manifest_not_index_jsonl() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let manifest_path = dir.path().join(".bo/manifest.json");

    // Seed the manifest with one leaf (mirrors seed_for_collect + a pushed
    // leaf) but write NO index. The dedup path (duplicate_file) must consult
    // the manifest, not the old index.
    let m = Manifest {
        tree: TreeMeta {
            name: "manifest-dedup-tree".to_string(),
            created_at: Timestamp::parse("2026-05-19T12:00:00Z").unwrap(),
            last_compiled_at: None,
        },
        leaves: vec![crate::domain::Leaf {
            slug: Slug::parse("already-collected").unwrap(),
            file: "already-collected.md".to_string(),
            title: Some(Title::parse("Already").unwrap()),
            url: Url::parse("https://example.com/article").unwrap(),
            collected_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            summary: None,
        }],
        branches: Vec::new(),
    };
    crate::engine::manifest::write(&manifest_path, &m).unwrap();

    // Verify duplicate_file (the dedup helper) finds it via manifest only.
    let existing = duplicate_file("https://example.com/article", dir.path()).unwrap();
    assert_eq!(existing.as_deref(), Some("already-collected.md"));

    // Sanity: a different URL is not flagged.
    let none = duplicate_file("https://example.com/other", dir.path()).unwrap();
    assert!(none.is_none());
}
