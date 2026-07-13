use super::{
    compile_error_payload, degenerate_result_warning, plan, render_human, repair, CompileOptions,
    CompileResult, CompileRunMode,
};
use crate::cli::json;
use crate::domain::manifest::{Manifest, TreeMeta};
use crate::domain::{Branch, Leaf, Title, Url};
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
    crate::engine::manifest::write(&crate::domain::tree::manifest_path(&tree.path), manifest)
        .unwrap();
}

fn read_manifest(dir: &Path) -> Manifest {
    let manifest_path = crate::domain::tree::manifest_path(dir);
    crate::engine::manifest::read(&manifest_path).unwrap()
}

fn leaf_record(slug: &str, file: &str, title: &str, collected_at: &str) -> Leaf {
    Leaf {
        slug: Slug::generate(slug, ""),
        file: file.to_string(),
        title: Title::parse(title).ok(),
        url: Url::parse("https://example.com").unwrap(),
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
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

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
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

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
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

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
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

    assert_eq!(notifications.len(), 1);
    assert!(notifications[0].contains("pruned 2 orphan"));
    assert_eq!(read_manifest(dir.path()).leaves.len(), 0);
}

#[test]
fn compile_result_notifications_skipped_from_json() {
    let result = CompileResult {
        status: "compiled".to_string(),
        reason: None,
        mode: Some(super::CompileRunMode::Full),
        model: Some("gpt-4.1".to_string()),
        branches: vec![super::BranchResult {
            slug: "test-branch".to_string(),
            title: "Test Branch".to_string(),
            leaf_count: 2,
        }],
        leaves_processed: 2,
        leaves_skipped: Vec::new(),
        notifications: vec!["pruned 3 orphan leaf records".to_string()],
        warnings: Vec::new(),
    };

    let encoded = json::success_string("compile", &result, Vec::new()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["ok"], true);
    assert!(parsed["data"]["notifications"].is_null());
}

#[test]
fn compile_result_warnings_skipped_from_json() {
    let result = CompileResult {
        status: "compiled".to_string(),
        reason: None,
        mode: Some(super::CompileRunMode::Full),
        model: Some("gpt-4.1".to_string()),
        branches: vec![super::BranchResult {
            slug: "test-branch".to_string(),
            title: "Test Branch".to_string(),
            leaf_count: 2,
        }],
        leaves_processed: 2,
        leaves_skipped: Vec::new(),
        notifications: Vec::new(),
        warnings: vec!["warning: title collision — shared".to_string()],
    };

    let encoded = json::success_string("compile", &result, Vec::new()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    // warnings are presentation (stderr), never part of the JSON envelope.
    assert!(parsed["data"]["warnings"].is_null());
}

fn branch_record(slug: &str, title: &str, leaf_slugs: &[&str]) -> Branch {
    Branch {
        slug: Slug::generate(slug, ""),
        file: format!("branch/{}.md", slug),
        title: Title::parse(title).unwrap(),
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
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

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
        model: None,
        branches: Vec::new(),
        leaves_processed: 0,
        leaves_skipped: Vec::new(),
        notifications: vec![
            "pruned 1 orphan leaf record (file missing, not in any branch)".to_string(),
        ],
        warnings: Vec::new(),
    };
    let mut stdout = Vec::new();
    render_human(&result, &mut stdout, "test-tree").unwrap();
    let output = String::from_utf8(stdout).unwrap();

    assert!(output.contains("test-tree is empty"));
    assert!(output.contains("\u{2192} pruned 1 orphan"));
}

// ── run-mode selection ─────────────────────────────────────────────────────

#[test]
fn select_run_mode_forces_full_when_no_branches_exist() {
    // A fresh tree (no branches) has nothing to incrementally update, so it
    // must compile full even without --all. Incremental mode against an empty
    // branch graph sends a prompt with no branch context but an incremental
    // response schema, so the LLM cannot produce valid updated_branches.
    let manifest = fresh_manifest("t", "2026-01-01T00:00:00Z", None);
    assert_eq!(
        plan::select_run_mode(
            CompileOptions {
                all: false,
                ..Default::default()
            },
            &manifest
        ),
        CompileRunMode::Full,
        "fresh tree with no branches must compile full even without --all"
    );
}

#[test]
fn select_run_mode_incremental_only_with_branches_and_no_all() {
    use crate::domain::Branch;
    use crate::domain::Title;

    let mut manifest = fresh_manifest("t", "2026-01-01T00:00:00Z", Some("2026-01-02T00:00:00Z"));
    manifest.branches.push(Branch {
        slug: Slug::generate("existing", ""),
        file: "branch/existing.md".to_string(),
        title: Title::parse("existing").unwrap(),
        created_at: Timestamp::parse("2026-01-02T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-01-02T00:00:00Z").unwrap(),
        leaves: vec![Slug::generate("a", "")],
    });

    assert_eq!(
        plan::select_run_mode(
            CompileOptions {
                all: false,
                ..Default::default()
            },
            &manifest
        ),
        CompileRunMode::Incremental,
        "tree with branches and no --all runs incremental"
    );
    assert_eq!(
        plan::select_run_mode(
            CompileOptions {
                all: true,
                ..Default::default()
            },
            &manifest
        ),
        CompileRunMode::Full,
        "--all always forces full"
    );
}

// ── context-mode selection ─────────────────────────────────────────────────

#[test]
fn ensure_compile_context_fits_errors_on_overflow() {
    use crate::cli::compile::execute::ensure_compile_context_fits;
    use crate::engine::llm::{Model, Provider};

    let model = Model::parse("gpt-4.1-mini", Provider::OpenAI).unwrap();

    let small = execute_prompt_tokens(64);
    assert!(ensure_compile_context_fits(&model, small).is_ok());

    let huge = execute_prompt_tokens(usize::MAX);
    assert!(
        ensure_compile_context_fits(&model, huge).is_err(),
        "overflow must error"
    );
}

/// Wrap a byte count into a token estimate comparable to what the compile
/// pipeline computes, so tests exercise the same fit-check path.
fn execute_prompt_tokens(prompt_bytes: usize) -> usize {
    crate::cli::compile::execute::estimate_compile_prompt_tokens(prompt_bytes)
}

#[test]
fn derived_compile_schema_requires_branches() {
    let schema =
        serde_json::to_value(crate::engine::schema::inline_schema_for::<CompileResponse>())
            .unwrap();
    let obj = schema.as_object().expect("top-level is object");
    assert_eq!(obj["additionalProperties"], false);
    let required: Vec<&str> = obj["required"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(required.contains(&"branches"));
}

#[test]
fn derived_incremental_compile_schema_requires_updated_and_new_branches() {
    let schema = serde_json::to_value(crate::engine::schema::inline_schema_for::<
        IncrementalCompileResponse,
    >())
    .unwrap();
    let obj = schema.as_object().expect("top-level is object");
    assert_eq!(obj["additionalProperties"], false);
    let required: Vec<&str> = obj["required"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(required.contains(&"updated_branches"));
    assert!(required.contains(&"new_branches"));
}

// ── leaf reference fidelity ───────────────────────────────────────────────

use super::parse::{
    parse_and_validate_with_input_size, CompileResponse, IncrementalCompileResponse,
};
use super::plan::LoadedLeaf;
use super::validation::leaf_resolver;
use super::CompileError;

fn loaded_leaf(slug: &str, title: &str) -> LoadedLeaf {
    LoadedLeaf {
        slug: slug.to_string(),
        filename: format!("{}.md", slug),
        title: title.to_string(),
        summary: None,
        body: format!("body of {}", title),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

/// Minimal valid full-compile response: one branch over the given leaf refs.
fn branch_response(leaves: &[&str]) -> String {
    let leaves_json = leaves
        .iter()
        .map(|l| format!("\"{}\"", l))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"branches":[{{"title":"Concept","body":"body text","leaves":[{}]}}]}}"#,
        leaves_json
    )
}

#[test]
fn valid_leaf_reference_map_resolves_by_filename_stem_and_unique_title() {
    let leaves = vec![
        loaded_leaf("alpha-concept", "Alpha Concept"),
        loaded_leaf("beta-thing", "Beta Thing"),
    ];
    let lookup = leaf_resolver(&leaves);

    assert!(lookup.collisions.is_empty());
    // filename, stem (= slug), lowercased title, slugified title all resolve.
    assert_eq!(
        lookup.map.get("alpha-concept.md"),
        Some(&"alpha-concept.md".to_string())
    );
    assert_eq!(
        lookup.map.get("alpha-concept"),
        Some(&"alpha-concept.md".to_string())
    );
    assert_eq!(
        lookup.map.get("alpha concept"),
        Some(&"alpha-concept.md".to_string())
    );
    assert_eq!(
        lookup.map.get("beta-thing.md"),
        Some(&"beta-thing.md".to_string())
    );
}

#[test]
fn valid_leaf_reference_map_drops_ambiguous_title_keys() {
    // Two leaves share a title → the title key is removed so a title reference
    // fails validation rather than silently resolving to the wrong leaf.
    let leaves = vec![
        loaded_leaf("gamma-one", "Shared Topic"),
        loaded_leaf("gamma-two", "Shared Topic"),
    ];
    let lookup = leaf_resolver(&leaves);

    assert!(
        !lookup.collisions.is_empty(),
        "expected a collision warning"
    );
    assert!(
        !lookup.map.contains_key("shared topic"),
        "ambiguous title key must be absent"
    );
    // Slugs (stems) stay unique and resolvable.
    assert_eq!(
        lookup.map.get("gamma-one"),
        Some(&"gamma-one.md".to_string())
    );
    assert_eq!(
        lookup.map.get("gamma-two"),
        Some(&"gamma-two.md".to_string())
    );
}

#[test]
fn collision_warnings_captured_as_data_not_printed() {
    // Two leaves share a title → the collision is recorded as a warning string
    // (previously eprintln'd inside validate). Validation still succeeds when a
    // branch references the leaves by their unique slug/filename.
    let leaves = vec![
        loaded_leaf("gamma-one", "Shared Topic"),
        loaded_leaf("gamma-two", "Shared Topic"),
    ];
    let mut warnings = Vec::new();
    parse_and_validate_with_input_size(
        &branch_response(&["gamma-one", "gamma-two.md"]),
        &leaves,
        1024,
        &mut warnings,
    )
    .expect("refs by unique slug/filename must resolve despite title collision");

    assert!(
        warnings
            .iter()
            .any(|w| w.contains("warning: title collision") && w.contains("Shared Topic")),
        "expected a title-collision warning captured as data: {:?}",
        warnings
    );
}

#[test]
fn parse_resolves_leaf_references_by_slug_filename_and_title() {
    let leaves = vec![
        loaded_leaf("alpha-concept", "Alpha Concept"),
        loaded_leaf("beta-thing", "Beta Thing"),
    ];

    // slug/stem + filename both resolve and normalize to the canonical filename.
    let plan = parse_and_validate_with_input_size(
        &branch_response(&["alpha-concept", "beta-thing.md"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .expect("refs by slug and filename must resolve");
    assert_eq!(
        plan.branches[0].leaves,
        vec!["alpha-concept.md", "beta-thing.md"]
    );

    // unique title resolves (case-insensitive).
    parse_and_validate_with_input_size(
        &branch_response(&["Alpha Concept", "Beta Thing"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .expect("refs by unique title must resolve");
}

#[test]
fn parse_rejects_invented_leaf_reference() {
    let leaves = vec![
        loaded_leaf("alpha-concept", "Alpha Concept"),
        loaded_leaf("beta-thing", "Beta Thing"),
    ];
    let err = parse_and_validate_with_input_size(
        &branch_response(&["alpha-concept", "invented-name"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, CompileError::Validation(ref msg) if msg.contains("unknown leaf")),
        "invented leaf reference must be a validation error: {:?}",
        err
    );
}

#[test]
fn parse_rejects_ambiguous_title_reference() {
    let leaves = vec![
        loaded_leaf("gamma-one", "Shared Topic"),
        loaded_leaf("gamma-two", "Shared Topic"),
        loaded_leaf("alpha-concept", "Alpha Concept"),
    ];
    let err = parse_and_validate_with_input_size(
        &branch_response(&["Shared Topic", "alpha-concept"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, CompileError::Validation(ref msg) if msg.contains("unknown leaf")),
        "ambiguous title reference must fail validation, not silently resolve: {:?}",
        err
    );
}

// ── leaf multi-membership ────────────────────────────────────────────────────

#[test]
fn derived_compile_schema_has_no_ref_or_defs_or_schema_key() {
    let schema =
        serde_json::to_value(crate::engine::schema::inline_schema_for::<CompileResponse>())
            .unwrap();
    let json_str = schema.to_string();
    assert!(!json_str.contains("\"$schema\""));
    assert!(!json_str.contains("\"definitions\""));
    assert!(!json_str.contains("\"$ref\""));
}

#[test]
fn derived_incremental_compile_schema_has_no_ref_or_defs_or_schema_key() {
    let schema = serde_json::to_value(crate::engine::schema::inline_schema_for::<
        IncrementalCompileResponse,
    >())
    .unwrap();
    let json_str = schema.to_string();
    assert!(!json_str.contains("\"$schema\""));
    assert!(!json_str.contains("\"definitions\""));
    assert!(!json_str.contains("\"$ref\""));
}

#[test]
fn build_manifest_delta_allows_one_leaf_in_multiple_branches() {
    use super::plan::build_manifest_delta;
    use super::validation::{CompilePlan, ValidatedBranch};
    use super::CompileRunMode;

    // One leaf participates in two cross-cutting concepts. The manifest model
    // stores branch→leaf as independent lists, so the same slug may appear in
    // several branches; the inverse is computed by branches_for_leaf.
    let plan = CompilePlan {
        branches: vec![
            ValidatedBranch {
                slug: "concept-a".to_string(),
                title: "Concept A".to_string(),
                body: "body a".to_string(),
                leaves: vec!["shared-leaf.md".to_string(), "alpha.md".to_string()],
            },
            ValidatedBranch {
                slug: "concept-b".to_string(),
                title: "Concept B".to_string(),
                body: "body b".to_string(),
                leaves: vec!["shared-leaf.md".to_string(), "beta.md".to_string()],
            },
        ],
    };

    let current = fresh_manifest("t", "2026-01-01T00:00:00Z", None);
    let ts = Timestamp::parse("2026-06-28T00:00:00Z").unwrap();
    let delta = build_manifest_delta(&current, &plan, CompileRunMode::Full, &ts).unwrap();

    assert_eq!(delta.new_manifest.branches.len(), 2);
    assert_eq!(
        delta.branches_created.len(),
        2,
        "both branches are new in Full mode"
    );

    let shared_slug = Slug::parse("shared-leaf").unwrap();
    let containing: Vec<&str> = delta
        .new_manifest
        .branches_for_leaf(&shared_slug)
        .iter()
        .map(|b| b.slug.as_str())
        .collect();
    assert_eq!(
        containing,
        vec!["concept-a", "concept-b"],
        "a leaf must be allowed in multiple branches"
    );
}

// ── full-mode parse validation gaps ──────────────────────────────────────

#[test]
fn parse_full_rejects_empty_title() {
    let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
    let json = r#"{"branches":[{"title":"","body":"some body","leaves":["a","b"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("empty title")));
}

#[test]
fn parse_full_rejects_empty_body() {
    let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
    let json = r#"{"branches":[{"title":"Concept","body":"","leaves":["a","b"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("empty body")));
}

#[test]
fn parse_full_rejects_duplicate_slug() {
    let leaves = vec![
        loaded_leaf("a", "A"),
        loaded_leaf("b", "B"),
        loaded_leaf("c", "C"),
        loaded_leaf("d", "D"),
    ];
    // Two branches with the same title → same slug → duplicate slug error
    let json = r#"{"branches":[{"title":"Same Thing","body":"body","leaves":["a","b"]},{"title":"Same Thing","body":"body","leaves":["c","d"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(
        matches!(err, CompileError::Validation(ref msg) if msg.contains("duplicate branch slug"))
    );
}

#[test]
fn parse_full_rejects_single_leaf_branch() {
    let leaves = vec![
        loaded_leaf("a", "A"),
        loaded_leaf("b", "B"),
        loaded_leaf("c", "C"),
    ];
    let json = r#"{"branches":[{"title":"Concept","body":"body","leaves":["a"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("at least 2 leaves")));
}

// ── incremental-mode parse validation ─────────────────────────────────────

use super::parse::parse_and_validate_incremental_with_input_size;

/// Minimal valid incremental response helper.
fn incremental_response(updated: &str, new: &str) -> String {
    format!(
        r#"{{"updated_branches":{},"new_branches":{}}}"#,
        updated, new
    )
}

/// Set up a tree on disk with 4 leaves, 1 existing branch (covering the 2 older
/// leaves), and fresh leaf files. Returns config, manifest, and loaded leaves.
fn setup_incremental_tree(dir: &Path) -> (SeededConfig, Manifest, Vec<LoadedLeaf>) {
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));
    // leaf-a, leaf-b: new (collected after last_compile)
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
    // leaf-c, leaf-d: existing (collected before last_compile, already branched)
    manifest.leaves.push(leaf_record(
        "leaf-c",
        "leaf-c.md",
        "Leaf C",
        "2026-01-05T00:00:00Z",
    ));
    manifest.leaves.push(leaf_record(
        "leaf-d",
        "leaf-d.md",
        "Leaf D",
        "2026-01-05T00:00:00Z",
    ));
    manifest.branches.push(branch_record(
        "existing",
        "Existing Branch",
        &["leaf-c", "leaf-d"],
    ));

    write_leaf(dir, "leaf-a.md", "---\ntitle: Leaf A\n---\n\nbody a\n");
    write_leaf(dir, "leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody b\n");
    write_leaf(dir, "leaf-c.md", "---\ntitle: Leaf C\n---\n\nbody c\n");
    write_leaf(dir, "leaf-d.md", "---\ntitle: Leaf D\n---\n\nbody d\n");

    std::fs::create_dir_all(dir.join("branch")).unwrap();
    std::fs::write(
        dir.join("branch/existing.md"),
        "---\ntitle: Existing Branch\n---\n\n# Existing Branch\n\nbody\n",
    )
    .unwrap();

    write_manifest(dir, &manifest);

    let cfg = seeded_config(dir);
    let loaded = vec![
        loaded_leaf("leaf-a", "Leaf A"),
        loaded_leaf("leaf-b", "Leaf B"),
        loaded_leaf("leaf-c", "Leaf C"),
        loaded_leaf("leaf-d", "Leaf D"),
    ];
    (cfg, manifest, loaded)
}

#[test]
fn parse_incremental_update_preserves_existing_leaves_and_adds_new() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    // Update existing branch: preserves leaf-c, leaf-d and adds leaf-a
    let updated = r#"[{"slug":"existing","title":"Existing Branch","body":"updated body","leaves":["leaf-c","leaf-d","leaf-a"]}]"#;
    let new = r#"[]"#;
    let plan = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap();

    assert_eq!(plan.branches.len(), 1);
    assert_eq!(plan.branches[0].slug, "existing");
    assert!(plan.branches[0].leaves.contains(&"leaf-a.md".to_string()));
    assert!(plan.branches[0].leaves.contains(&"leaf-c.md".to_string()));
    assert!(plan.branches[0].leaves.contains(&"leaf-d.md".to_string()));
}

#[test]
fn parse_incremental_update_dropping_existing_leaf_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    // Update that omits leaf-d (an existing leaf) — not allowed
    let updated = r#"[{"slug":"existing","title":"Existing Branch","body":"body","leaves":["leaf-c","leaf-a"]}]"#;
    let new = r#"[]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, CompileError::Validation(ref msg) if msg.contains("dropped existing leaf"))
    );
}

#[test]
fn parse_incremental_new_branch_without_new_leaf_is_dropped() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    // New branch references only old leaves (no new leaf integrated)
    let updated = r#"[]"#;
    let new = r#"[{"title":"Reorganised","body":"body","leaves":["leaf-c","leaf-d"]}]"#;
    let plan = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap();

    // Branch is silently dropped (no new leaves)
    assert!(
        plan.branches.is_empty(),
        "new branch without new leaf must be dropped"
    );
}

#[test]
fn parse_incremental_update_unknown_branch_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    let updated =
        r#"[{"slug":"nonexistent","title":"Whatever","body":"body","leaves":["leaf-a","leaf-b"]}]"#;
    let new = r#"[]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("unknown branch")));
}

#[test]
fn parse_incremental_title_change_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    let updated = r#"[{"slug":"existing","title":"Different Title","body":"body","leaves":["leaf-c","leaf-d","leaf-a"]}]"#;
    let new = r#"[]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("changed title")));
}

#[test]
fn parse_incremental_new_branch_empty_title_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    let updated = r#"[]"#;
    let new = r#"[{"title":"","body":"body","leaves":["leaf-a","leaf-b"]}]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("empty title")));
}

#[test]
fn parse_incremental_new_branch_empty_body_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    let updated = r#"[]"#;
    let new = r#"[{"title":"Valid Title","body":"","leaves":["leaf-a","leaf-b"]}]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("empty body")));
}

#[test]
fn parse_incremental_update_with_no_new_leaf_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    // Update that adds no new leaf
    let updated = r#"[{"slug":"existing","title":"Existing Branch","body":"body","leaves":["leaf-c","leaf-d"]}]"#;
    let new = r#"[]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(
        matches!(err, CompileError::Validation(ref msg) if msg.contains("no newly processed leaf"))
    );
}

#[test]
fn parse_incremental_insufficient_leaves_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

    // New branch with only 1 leaf
    let updated = r#"[]"#;
    let new = r#"[{"title":"Solo","body":"body","leaves":["leaf-a"]}]"#;
    let err = parse_and_validate_incremental_with_input_size(
        &incremental_response(updated, new),
        &cfg,
        &loaded,
        4096,
        &mut Vec::new(),
    )
    .unwrap_err();
    assert!(matches!(err, CompileError::Validation(ref msg) if msg.contains("at least 2 leaves")));
}

// ── repair: branch frontmatter consistency ───────────────────────────────

#[test]
fn repair_stale_branches_fixes_branch_frontmatter() {
    let dir = TempDir::new().unwrap();
    let mut manifest = fresh_manifest("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));

    // 3 leaves, all new (collected after last_compile)
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

    // One branch covering all 3 leaves
    manifest.branches.push(branch_record(
        "test-branch",
        "Test Branch",
        &["leaf-a", "leaf-b", "leaf-c"],
    ));

    // Write leaf files (leaf-a intentionally absent/deleted)
    write_leaf(
        dir.path(),
        "leaf-b.md",
        "---\ntitle: Leaf B\n---\n\nbody b\n",
    );
    write_leaf(
        dir.path(),
        "leaf-c.md",
        "---\ntitle: Leaf C\n---\n\nbody c\n",
    );

    // Write branch file with 3 leaves in frontmatter
    std::fs::create_dir_all(dir.path().join("branch")).unwrap();
    let branch_content = "---\ntitle: Test Branch\ncreated_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\nleaves:\n- leaf-a.md\n- leaf-b.md\n- leaf-c.md\n---\n\n# Test Branch\n\nBody text with reference to Leaf A\n";
    std::fs::write(dir.path().join("branch/test-branch.md"), branch_content).unwrap();

    write_manifest(dir.path(), &manifest);

    let cfg = seeded_config(dir.path());
    let notifications = repair::repair_stale_branches(&cfg, &manifest)
        .expect("repair should succeed")
        .notifications;

    // Notification should mention frontmatter repair
    assert!(
        notifications
            .iter()
            .any(|n| n.contains("frontmatter repaired")),
        "expected frontmatter repair notification in: {:?}",
        notifications
    );

    // Branch file frontmatter leaves: should have 2 entries (leaf-b, leaf-c),
    // not 3.
    let repaired = std::fs::read_to_string(dir.path().join("branch/test-branch.md")).unwrap();
    assert!(repaired.contains("- leaf-b.md"));
    assert!(repaired.contains("- leaf-c.md"));
    assert!(!repaired.contains("- leaf-a.md"));

    // Body is preserved (stale reference to leaf-a stays)
    assert!(repaired.contains("reference to Leaf A"));

    // Manifest branch leaves should match: 2 entries
    let repaired_manifest = read_manifest(dir.path());
    let branch = repaired_manifest
        .branches
        .iter()
        .find(|b| b.slug.as_str() == "test-branch")
        .unwrap();
    assert_eq!(branch.leaves.len(), 2);
}

// ── degenerate result warning ────────────────────────────────────────────

use super::BranchResult;

fn branch_result(slug: &str, leaf_count: usize) -> BranchResult {
    BranchResult {
        slug: slug.to_string(),
        title: slug.to_string(),
        leaf_count,
    }
}

#[test]
fn degenerate_warning_when_single_branch_for_many_leaves() {
    // gpt-4.1 at 64 leaves silently produced 1 branch / 2 leaves.
    // <2 branches for >20 leaves is degenerate.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[branch_result("catch-all", 2)],
        64,
    );
    let msg = warning.expect("expected a degenerate warning");
    assert!(msg.contains("degenerate compile result"));
    assert!(msg.contains("1 branch"));
    assert!(msg.contains("64 leaves"));
}

#[test]
fn degenerate_warning_when_most_leaves_unbranched() {
    // 3 branches covering only 5 of 30 leaves = 83% unbranched.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[
            branch_result("a", 2),
            branch_result("b", 2),
            branch_result("c", 1),
        ],
        30,
    );
    let msg = warning.expect("expected a degenerate warning");
    assert!(msg.contains("degenerate compile result"));
    assert!(msg.contains("25 of 30 leaves unbranched"));
}

#[test]
fn no_degenerate_warning_for_normal_full_compile() {
    // 3 branches covering 28 of 30 leaves = 7% unbranched, well within bounds.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[
            branch_result("a", 10),
            branch_result("b", 10),
            branch_result("c", 8),
        ],
        30,
    );
    assert!(warning.is_none());
}

#[test]
fn no_degenerate_warning_for_small_corpus() {
    // 20 leaves or fewer are never warned about, even with 0 branches.
    let warning = degenerate_result_warning(Some(CompileRunMode::Full), &[], 20);
    assert!(warning.is_none());
}

#[test]
fn no_degenerate_warning_for_incremental_mode() {
    // Incremental mode never produces degenerate warnings — it naturally
    // produces fewer branches by design.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Incremental),
        &[branch_result("single", 2)],
        64,
    );
    assert!(warning.is_none());
}

#[test]
fn degenerate_warning_low_coverage_ratio() {
    // 2 branches, 66 leaves processed, but branches only claim 15 leaves
    // total (7+8). Coverage = 15/66 ≈ 0.23, below the 0.30 threshold.
    // The unbranched heuristic (77% < 80%) does NOT fire, so this
    // exercises the new coverage-ratio path exclusively.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[branch_result("concept-a", 7), branch_result("concept-b", 8)],
        66,
    );
    let msg = warning.expect("expected a degenerate warning from low coverage ratio");
    assert!(msg.contains("degenerate compile result"));
    assert!(msg.contains("only 15 of 66 leaves placed in branches"));
}

#[test]
fn no_degenerate_warning_for_healthy_coverage() {
    // 3 branches covering 26 of 30 leaves = 87% coverage, 13% unbranched.
    // Both the unbranched (>80%) and coverage (<30%) guards pass through.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[
            branch_result("a", 10),
            branch_result("b", 9),
            branch_result("c", 7),
        ],
        30,
    );
    assert!(warning.is_none());
}

#[test]
fn degenerate_warning_single_branch_regression() {
    // Regression guard: branch_count < 2 with >20 leaves still warns.
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[branch_result("catch-all", 2)],
        64,
    );
    assert!(warning.is_some());
}

#[test]
fn degenerate_warning_unbranched_regression() {
    // Regression guard: >80% unbranched still warns.
    // 3 branches, 5 of 30 leaves branched → 25 unbranched (83%).
    let warning = degenerate_result_warning(
        Some(CompileRunMode::Full),
        &[
            branch_result("a", 2),
            branch_result("b", 2),
            branch_result("c", 1),
        ],
        30,
    );
    assert!(warning.is_some());
}

#[test]
fn compile_error_payload_routes_terminal_errors() {
    use std::time::Duration;
    let slugs: &[String] = &[];
    let duration = Duration::from_millis(10);

    // Validation keeps its own shape: validation_failures, no error field.
    let payload = compile_error_payload(
        CompileRunMode::Full,
        slugs,
        &CompileError::Validation("branch #1 has empty title".to_string()),
        duration,
    )
    .expect("validation is journaled");
    assert_eq!(
        payload.validation_failures,
        vec!["branch #1 has empty title".to_string()]
    );
    assert!(payload.error.is_none());

    // LLM/provider failures: error field, empty deltas.
    let llm_errors = [
        CompileError::Truncated,
        CompileError::ContentFilter,
        CompileError::Llm("upstream timeout".to_string()),
        CompileError::ContextOverflow {
            model: "gpt-4.1".to_string(),
            estimated_tokens: Some(200_000),
            context_tokens: Some(128_000),
        },
    ];
    for error in &llm_errors {
        let payload = compile_error_payload(CompileRunMode::Full, slugs, error, duration)
            .expect("LLM/provider error is journaled");
        assert!(payload.validation_failures.is_empty());
        let err = payload.error.expect("error field present");
        assert!(!err.code.is_empty());
        assert!(!err.message.is_empty());
    }

    // Infrastructure / dry-run / agent failures are not compile verdicts.
    for error in [
        CompileError::Io("disk full".to_string()),
        CompileError::Busy("locked".to_string()),
        CompileError::DryRunBlocked("stale".to_string()),
        CompileError::AgentFailed {
            message: "limit".to_string(),
            turns: 0,
            tool_calls: 0,
            usage: None,
            last_error: None,
        },
    ] {
        assert!(
            compile_error_payload(CompileRunMode::Full, slugs, &error, duration).is_none(),
            "{:?} should not be journaled",
            error
        );
    }
}
