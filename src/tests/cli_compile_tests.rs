use super::{plan, render_human, CompileOptions, CompileResult, CompileRunMode};
use crate::domain::manifest::{BranchRecord, LeafRecord, Manifest, TreeMeta};
use crate::domain::{Slug, Timestamp};
use crate::engine::config::SeededConfig;
use std::collections::HashSet;
use std::path::Path;
use tempfile::TempDir;

// ── helpers ───────────────────────────────────────────────────────────────────

fn seeded_config(dir: &Path) -> SeededConfig {
    let tree_cfg = crate::domain::tree::TreeConfig {
        path: dir.to_path_buf(),
        name: "test-tree".to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
    };
    SeededConfig::new(crate::engine::config::Config::default(), tree_cfg)
}

fn write_manifest(dir: &Path, manifest: &Manifest) {
    let tree = crate::domain::tree::Tree::from_config(&crate::domain::tree::TreeConfig {
        path: dir.to_path_buf(),
        name: "test-tree".to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
    });
    crate::domain::manifest::write(&crate::domain::tree::manifest_path(&tree.path), manifest)
        .unwrap();
}

fn read_manifest(dir: &Path) -> Manifest {
    let manifest_path = crate::domain::tree::manifest_path(dir);
    crate::domain::manifest::read(&manifest_path).unwrap()
}

fn leaf_record(slug: &str, file: &str, title: &str, collected_at: &str) -> LeafRecord {
    LeafRecord {
        slug: Slug::generate(slug, ""),
        file: file.to_string(),
        title: title.to_string(),
        url: ("https://example.com").to_string(),
        collected_at: Timestamp::parse(collected_at).unwrap(),
        summary: Some("summary text".to_string()),
    }
}

fn fresh_manifest(name: &str, created_at: &str, last_compiled_at: Option<&str>) -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: name.to_string(),
            created_at: Timestamp::parse(created_at).unwrap(),
            last_compiled_at: last_compiled_at.map(|s| Timestamp::parse(s).unwrap()),
        },
        leaves: Vec::new(),
        branches: Vec::new(),
    }
}

fn write_leaf(dir: &Path, file: &str, content: &str) {
    let path = dir.join(file);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[test]
fn missing_unbranched_new_leaf_is_pruned_not_error() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-02-01T00:00:00Z"));
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-03-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-03-01T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody\n");
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications =
        plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 1 orphan"));

    let repaired = read_manifest(dir.path());
    assert_eq!(repaired.leaves.len(), 1);
    assert_eq!(repaired.leaves[0].slug.as_str(), "leaf-b");
}

#[test]
fn missing_unbranched_leaf_never_compiled_is_pruned() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", None);
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-01-15T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody\n");
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications =
        plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 1 orphan"));
    assert_eq!(read_manifest(dir.path()).leaves.len(), 1);
}

#[test]
fn repair_with_no_missing_files_has_empty_notifications() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", None);
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    write_leaf(dir.path(), "leaf-a.md", "---\ntitle: Leaf A\n---\n\nbody\n");
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications =
        plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert!(notifications.is_empty());
}

#[test]
fn all_leaves_deleted_manifest_repaired_to_empty() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-02-01T00:00:00Z"));
    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-03-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-03-01T00:00:00Z",
    ));
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications =
        plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 2 orphan"));
    assert_eq!(read_manifest(dir.path()).leaves.len(), 0);
}

#[test]
fn compile_result_notifications_serialized_and_omitted_when_empty() {
    let result = CompileResult {
        status: "noop".to_string(),
        reason: Some("empty_tree".to_string()),
        mode: None,
        context_mode: None,
        model: None,
        branches: Vec::new(),
        leaves_processed: 0,
        leaves_skipped: Vec::new(),
        notifications: vec!["pruned 3 orphan leaf records".to_string()],
    };

    let json = serde_json::to_string_pretty(&result).unwrap();
    assert!(json.contains("notifications"));
    assert!(json.contains("pruned 3 orphan leaf records"));

    let result_no_notifications = CompileResult {
        notifications: Vec::new(),
        ..result
    };
    let json_empty = serde_json::to_string_pretty(&result_no_notifications).unwrap();
    assert!(!json_empty.contains("notifications"));
}

fn branch_record(slug: &str, title: &str, leaf_slugs: &[&str]) -> BranchRecord {
    BranchRecord {
        slug: Slug::generate(slug, ""),
        file: format!("branches/{}.md", slug),
        title: title.to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        leaves: leaf_slugs.iter().map(|s| Slug::generate(s, "")).collect(),
    }
}

#[test]
fn repair_notifications_include_branch_repair_and_removal_messages() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));

    manifest.leaves.push(leaf_record(
        "leaf-a",
        "leaf-a.md",
        "Leaf A",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-b",
        "leaf-b.md",
        "Leaf B",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-c",
        "leaf-c.md",
        "Leaf C",
        "2026-02-01T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-d",
        "leaf-d.md",
        "Leaf D",
        "2026-02-01T00:00:00Z",
    ));

    manifest
        .branches
        .push(branch_record("branch-1", "Branch 1", &["leaf-a", "leaf-b"]));
    manifest.branches.push(branch_record(
        "branch-2",
        "Branch 2",
        &["leaf-a", "leaf-b", "leaf-c"],
    ));
    manifest
        .branches
        .push(branch_record("branch-3", "Branch 3", &["leaf-c", "leaf-d"]));

    // Write only leaf-a and leaf-b; leaf-c and leaf-d are missing.
    write_leaf(
        dir.path(),
        "leaf-a.md",
        "---\ntitle: Leaf A\n---\n\nbody a\n",
    );
    write_leaf(
        dir.path(),
        "leaf-b.md",
        "---\ntitle: Leaf B\n---\n\nbody b\n",
    );
    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications =
        plan::repair_stale_branches(&cfg, &manifest).expect("repair should succeed");

    // Messages should include branch repair and removal, not just prune.
    let notification_set: HashSet<&str> = notifications.iter().map(String::as_str).collect();
    assert!(
        notification_set
            .iter()
            .any(|n| n.contains("repaired 1 branch")),
        "expected 'repaired 1 branch' in notifications: {:?}",
        notifications
    );
    assert!(
        notification_set
            .iter()
            .any(|n| n.contains("removed 1 stale branch")),
        "expected 'removed 1 stale branch' in notifications: {:?}",
        notifications
    );

    // branch-1 (all leaves present): no repair needed, stays in manifest
    // branch-2 (leaf-c missing): repaired, stays at 2 leaves
    // branch-3 (both leaves missing): removed
    let repaired = read_manifest(dir.path());
    assert_eq!(repaired.branches.len(), 2);
    let branch_slugs: HashSet<&str> = repaired.branches.iter().map(|b| b.slug.as_str()).collect();
    assert!(branch_slugs.contains("branch-1"));
    assert!(branch_slugs.contains("branch-2"));
    assert!(!branch_slugs.contains("branch-3"));
}

#[test]
fn human_output_includes_notifications() {
    let result = CompileResult {
        status: "noop".to_string(),
        reason: Some("empty_tree".to_string()),
        mode: None,
        context_mode: None,
        model: None,
        branches: Vec::new(),
        leaves_processed: 0,
        leaves_skipped: Vec::new(),
        notifications: vec![
            "pruned 1 orphan leaf record (file missing, not in any branch)".to_string(),
        ],
    };
    let mut stdout = Vec::new();
    render_human(&result, &mut stdout, "test-tree").unwrap();
    let output = String::from_utf8(stdout).unwrap();

    assert!(output.contains("test-tree is empty"));
    assert!(output.contains("\u{2192} pruned 1 orphan"));
}

// ── run-mode selection (see #95) ─────────────────────────────────────────────

#[test]
fn select_run_mode_forces_full_when_no_branches_exist() {
    // A fresh tree (no branches) has nothing to incrementally update, so it
    // must compile full even without --all. Incremental mode against an empty
    // branch graph sends a prompt with no branch context but an incremental
    // response schema — the root of #87.
    let manifest = fresh_manifest("t", "2026-01-01T00:00:00Z", None);
    assert_eq!(
        plan::select_run_mode(CompileOptions { all: false }, &manifest),
        CompileRunMode::Full,
        "fresh tree with no branches must compile full even without --all"
    );
}

#[test]
fn select_run_mode_incremental_only_with_branches_and_no_all() {
    use crate::domain::manifest::BranchRecord;
    use crate::domain::Title;

    let mut manifest = fresh_manifest("t", "2026-01-01T00:00:00Z", Some("2026-01-02T00:00:00Z"));
    manifest.branches.push(BranchRecord {
        slug: Slug::generate("existing", ""),
        file: "branches/existing.md".to_string(),
        title: Title::from("existing"),
        created_at: Timestamp::parse("2026-01-02T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-01-02T00:00:00Z").unwrap(),
        leaves: vec![Slug::generate("a", "")],
    });

    assert_eq!(
        plan::select_run_mode(CompileOptions { all: false }, &manifest),
        CompileRunMode::Incremental,
        "tree with branches and no --all runs incremental"
    );
    assert_eq!(
        plan::select_run_mode(CompileOptions { all: true }, &manifest),
        CompileRunMode::Full,
        "--all always forces full"
    );
}

// ── context-mode selection (see #95) ─────────────────────────────────────────

#[test]
fn choose_context_mode_incremental_never_yields_full_corpus() {
    use crate::cli::compile::execute::choose_context_mode;
    use crate::cli::compile::CompileContextMode;
    use crate::engine::llm::{Model, Provider};

    let model = Model::parse("gpt-4.1-mini", Provider::OpenAI).unwrap();

    // Incremental with a fitting prompt yields IncrementalContext, never
    // FullCorpus (the broken optimization that paired the branch-less full
    // prompt with the incremental schema).
    let small = execute_prompt_tokens(64);
    assert_eq!(
        choose_context_mode(&model, CompileRunMode::Incremental, small).unwrap(),
        CompileContextMode::IncrementalContext,
    );

    // Incremental that overflows context errors rather than silently falling
    // back to FullCorpus.
    let huge = execute_prompt_tokens(usize::MAX);
    assert!(
        choose_context_mode(&model, CompileRunMode::Incremental, huge).is_err(),
        "incremental overflow must error, not fall back to full corpus"
    );

    // Full still yields FullCorpus when it fits.
    assert_eq!(
        choose_context_mode(&model, CompileRunMode::Full, small).unwrap(),
        CompileContextMode::FullCorpus,
    );
}

/// Wrap a byte count into a token estimate comparable to what the compile
/// pipeline computes, so tests exercise the same fit-check path.
fn execute_prompt_tokens(prompt_bytes: usize) -> usize {
    crate::cli::compile::execute::estimate_compile_prompt_tokens(prompt_bytes)
}
