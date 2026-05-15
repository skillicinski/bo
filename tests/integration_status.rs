// Integration tests for `bo status`.
//
// Tests the full CLI binary with $HOME override. Simulates tree states
// by directly constructing files (no network/LLM required).

use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn bo(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bo"));
    cmd.env("HOME", home);
    cmd
}

fn seed(home: &Path, output_dir: &Path) -> Output {
    bo(home)
        .args(["seed", output_dir.to_str().unwrap()])
        .output()
        .expect("failed to run bo seed")
}

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
    let content = format!(
        "---\ntitle: \"{slug}\"\nurl: {url}\ncollected_at: 2026-05-14T10:00:00Z\nupdated_at: 2026-05-14T10:00:00Z\n---\n\n# {slug}\n\nContent for {slug}.\n"
    );
    fs::write(tree_dir.join(&filename), content).unwrap();

    // Append to index
    let index_path = tree_dir.join(".bo/index.jsonl");
    let entry = format!("{{\"file\":\"{filename}\",\"title\":\"{slug}\",\"url\":\"{url}\"}}\n");
    let mut existing = fs::read_to_string(&index_path).unwrap_or_default();
    existing.push_str(&entry);
    fs::write(index_path, existing).unwrap();
}

fn write_branch(tree_dir: &Path, slug: &str, compiled_at: &str) {
    let branches_dir = tree_dir.join("branches");
    fs::create_dir_all(&branches_dir).unwrap();
    let content = format!(
        "---\ntitle: \"{slug}\"\ncompiled_at: {compiled_at}\nupdated_at: {compiled_at}\nleaves:\n  - some-leaf\n---\n\n# {slug}\n\nBranch body.\n"
    );
    fs::write(branches_dir.join(format!("{}.md", slug)), content).unwrap();
}

fn write_state(tree_dir: &Path, slugs: &[&str], timestamp: &str) {
    let compiled: HashMap<&str, &str> = slugs.iter().map(|s| (*s, timestamp)).collect();
    let state = serde_json::json!({ "compiled_leaves": compiled });
    fs::write(
        tree_dir.join(".bo/state.json"),
        serde_json::to_string_pretty(&state).unwrap(),
    )
    .unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn status_after_seed_shows_empty_tree() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = tmp.path().join("tree");

    let out = seed(tmp.path(), &tree_dir);
    assert!(out.status.success());

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
    let tree_dir = tmp.path().join("tree");

    seed(tmp.path(), &tree_dir);

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
    let tree_dir = tmp.path().join("tree");

    seed(tmp.path(), &tree_dir);

    write_leaf(&tree_dir, "leaf-a", "https://a.com");
    write_leaf(&tree_dir, "leaf-b", "https://b.com");

    // Simulate compile: write state + branch
    write_state(&tree_dir, &["leaf-a", "leaf-b"], "2026-05-15T10:00:00Z");
    write_branch(&tree_dir, "topic-one", "2026-05-15T10:00:00Z");

    let out = status(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("uncompiled"));
    assert!(stdout.contains("Branches:    1"));
    assert!(stdout.contains("2026-05-15T10:00:00Z"));
}

#[test]
fn status_detects_orphan_index_entry() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = tmp.path().join("tree");

    seed(tmp.path(), &tree_dir);

    write_leaf(&tree_dir, "exists", "https://exists.com");
    write_leaf(&tree_dir, "will-delete", "https://deleted.com");

    // Now delete the file but leave index entry
    fs::remove_file(tree_dir.join("will-delete.md")).unwrap();

    let out = status_json(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();

    let orphans = &json["data"]["health"]["orphan_index_entries"];
    assert_eq!(orphans.as_array().unwrap().len(), 1);
    assert_eq!(orphans[0]["file"], "will-delete.md");
}

#[test]
fn status_detects_missing_from_index() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = tmp.path().join("tree");

    seed(tmp.path(), &tree_dir);

    // Write a leaf file directly without going through collect (not in index)
    let content = "---\ntitle: \"stray\"\nurl: https://stray.com\ncollected_at: 2026-05-14T10:00:00Z\nupdated_at: 2026-05-14T10:00:00Z\n---\n\n# stray\n\nOrphaned leaf.\n";
    fs::write(tree_dir.join("stray.md"), content).unwrap();

    let out = status_json(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();

    let missing = &json["data"]["health"]["missing_from_index"];
    assert_eq!(missing.as_array().unwrap().len(), 1);
    assert_eq!(missing[0], "stray.md");
}

#[test]
fn status_json_is_valid_and_complete() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = tmp.path().join("tree");

    seed(tmp.path(), &tree_dir);
    write_leaf(&tree_dir, "test-leaf", "https://test.com");

    let out = status_json(tmp.path());
    assert!(out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: Value = serde_json::from_str(&stdout).unwrap();

    // Verify all top-level fields exist
    assert_eq!(json["ok"], true);
    assert_eq!(json["command"], "status");
    assert!(json["data"]["tree_name"].is_string());
    assert!(json["data"]["leaves"]["total"].is_number());
    assert!(json["data"]["leaves"]["uncompiled"].is_number());
    assert!(json["data"]["leaves"]["uncompiled_slugs"].is_array());
    assert!(json["data"]["branches"]["total"].is_number());
    assert!(json["data"]["size"]["bytes"].is_number());
    assert!(json["data"]["size"]["estimated_tokens"].is_number());
    assert!(json["data"]["health"]["orphan_index_entries"].is_array());
    assert!(json["data"]["health"]["missing_from_index"].is_array());
    assert!(json["data"]["hints"].is_array());
}

#[test]
fn status_not_seeded_exits_nonzero() {
    let tmp = TempDir::new().unwrap();
    // Don't seed — just run status

    let out = status(tmp.path());
    assert!(!out.status.success());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("seed"));
}
