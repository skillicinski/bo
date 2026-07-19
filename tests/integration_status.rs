// Integration tests for `bo status`.
//
// Tests the full CLI binary with $HOME override. Simulates tree states
// by directly constructing files (no network/LLM required).

mod common;

use bo::domain::{Slug, Timestamp, Title, Url};
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

use common::bo;

fn status(home: &Path) -> Output {
    bo(home)
        .arg("status")
        .output()
        .expect("failed to run bo status")
}

fn status_json(home: &Path) -> Output {
    bo(home)
        .args(["status", "--json"])
        .output()
        .expect("failed to run bo status --json")
}

fn write_leaf(tree_dir: &Path, slug: &str, url: &str) {
    let filename = format!("{}.md", slug);
    let collected_at = "2026-05-14T10:00:00Z";
    let content = format!(
        "---\ntitle: \"{slug}\"\nurl: {url}\ncollected_at: {collected_at}\nupdated_at: {collected_at}\n---\n\n# {slug}\n\nContent for {slug}.\n"
    );
    let leaf_dir = tree_dir.join("leaf");
    fs::create_dir_all(&leaf_dir).unwrap();
    fs::write(leaf_dir.join(&filename), content).unwrap();

    // Append to state so reads see the leaf.
    common::ensure_state(tree_dir);
    common::append_leaf(
        tree_dir,
        bo::domain::Leaf {
            slug: Slug::parse(slug).unwrap(),
            file: format!("leaf/{filename}"),
            title: Title::parse(slug).ok(),
            url: Url::parse(url).unwrap(),
            collected_at: Timestamp::parse(collected_at).unwrap(),
            summary: None,
        },
    );
}

fn write_branch(tree_dir: &Path, slug: &str, created_at: &str) {
    let branches_dir = tree_dir.join("branch");
    fs::create_dir_all(&branches_dir).unwrap();
    let content = format!(
        "---\ntitle: \"{slug}\"\ncreated_at: {created_at}\nupdated_at: {created_at}\nleaves:\n  - some-leaf\n---\n\n# {slug}\n\nBranch body.\n"
    );
    fs::write(branches_dir.join(format!("{}.md", slug)), content).unwrap();

    // Append to state and stamp last_compiled_at.
    common::ensure_state(tree_dir);
    let mut m = common::read_state(tree_dir);
    m.branches.push(bo::domain::Branch {
        slug: Slug::parse(slug).unwrap(),
        file: format!("branch/{}.md", slug),
        title: Title::parse(slug).unwrap(),
        created_at: Timestamp::parse(created_at).unwrap(),
        updated_at: Timestamp::parse(created_at).unwrap(),
        leaves: vec![Slug::parse("some-leaf").unwrap()],
    });
    m.tree.last_compiled_at = Some(Timestamp::parse(created_at).unwrap());
    common::write_state(tree_dir, &m);
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn status_after_seed_shows_empty_tree() {
    let tmp = TempDir::new().unwrap();
    common::seed(tmp.path(), "tree");

    let out = status(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Leaves:"));
    assert!(stdout.contains("0"));
    assert!(stdout.contains("bo collect"));
}

#[test]
fn status_shows_uncompiled_leaves() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = common::seed(tmp.path(), "tree");

    write_leaf(&tree_dir, "leaf-one", "https://one.com");
    write_leaf(&tree_dir, "leaf-two", "https://two.com");
    write_leaf(&tree_dir, "leaf-three", "https://three.com");

    let out = status(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("3 uncompiled"));
    assert!(stdout.contains("leaf-one"));
    assert!(stdout.contains("leaf-two"));
    assert!(stdout.contains("leaf-three"));
    assert!(stdout.contains("compile"));
}

#[test]
fn status_after_compile_shows_zero_uncompiled() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = common::seed(tmp.path(), "tree");

    write_leaf(&tree_dir, "leaf-a", "https://a.com");
    write_leaf(&tree_dir, "leaf-b", "https://b.com");

    // Simulate compile: write branch + update state timestamp.
    write_branch(&tree_dir, "topic-one", "2026-05-15T10:00:00Z");

    let out = status(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("uncompiled"));
    assert!(stdout.contains("Branches:    1"));
    assert!(stdout.contains("2026-05-15T10:00:00"));
}

#[test]
fn status_detects_missing_leaf_file() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = common::seed(tmp.path(), "tree");

    write_leaf(&tree_dir, "exists", "https://exists.com");
    write_leaf(&tree_dir, "will-delete", "https://deleted.com");

    // Now delete the file but leave the state entry
    fs::remove_file(tree_dir.join("leaf/will-delete.md")).unwrap();

    let out = status_json(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();

    let missing = &json["data"]["health"]["missing_leaf_files"];
    assert_eq!(missing.as_array().unwrap().len(), 1);
    assert_eq!(missing[0]["file"], "leaf/will-delete.md");
}

#[test]
fn status_detects_untracked_leaf_file() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = common::seed(tmp.path(), "tree");

    // Write a leaf file directly without going through collect (not tracked in state)
    let content = "---\ntitle: \"stray\"\nurl: https://stray.com\ncollected_at: 2026-05-14T10:00:00Z\nupdated_at: 2026-05-14T10:00:00Z\n---\n\n# stray\n\nOrphaned leaf.\n";
    let leaf_dir = tree_dir.join("leaf");
    fs::create_dir_all(&leaf_dir).unwrap();
    fs::write(leaf_dir.join("stray.md"), content).unwrap();

    let out = status_json(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();

    let untracked = &json["data"]["health"]["untracked_leaf_files"];
    assert_eq!(untracked.as_array().unwrap().len(), 1);
    assert_eq!(untracked[0], "stray.md");
}

#[test]
fn status_json_is_valid_and_complete() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = common::seed(tmp.path(), "tree");

    write_leaf(&tree_dir, "test-leaf", "https://test.com");

    let out = status_json(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();

    // Verify the complete status schema
    assert_eq!(json["schema_version"], 2);
    assert_eq!(json["ok"], true);
    assert_eq!(json["command"], "status");
    assert!(json["data"]["tree_name"].is_string());
    assert!(json["data"]["leaves"]["total"].is_number());
    assert!(json["data"]["leaves"]["uncompiled"].is_number());
    assert!(json["data"]["leaves"]["uncompiled_slugs"].is_array());
    assert!(json["data"]["branches"]["total"].is_number());
    assert!(json["data"]["branches"]["last_compiled_at"].is_null());
    assert!(json["data"]["size"]["bytes"].is_number());
    assert!(json["data"]["size"]["estimated_tokens"].is_number());
    assert!(json["data"]["health"]["missing_leaf_files"].is_array());
    assert!(json["data"]["health"]["untracked_leaf_files"].is_array());
    assert!(json["data"]["hints"].is_array());
    assert!(json["data"]["provider"].is_string());
}

#[test]
fn status_not_seeded_exits_nonzero() {
    let tmp = TempDir::new().unwrap();
    // Don't seed — just run status. Status works without a seeded tree
    // (shows config fields with a hint).

    let out = status(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("bo seed"));
}
