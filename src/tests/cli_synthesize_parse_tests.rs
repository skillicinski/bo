use crate::cli::synthesize::parse::{
    parse_and_validate_incremental_with_input_size, parse_and_validate_with_input_size,
    BranchSynthesisResponse, IncrementalSynthesisResponse,
};
use crate::cli::synthesize::plan::LoadedLeaf;
use crate::cli::synthesize::types::SynthesisError;
use crate::cli::synthesize::validation::leaf_resolver;
use crate::domain::slug::Slug;
use crate::domain::state::{TreeMetadata, TreeState};
use crate::domain::{Branch, Leaf, Timestamp, Title, Url};
use crate::engine::config::SeededConfig;
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

fn write_state(dir: &Path, state: &TreeState) {
    let tree = crate::domain::tree::Tree::from_config(&crate::domain::tree::TreeConfig {
        path: dir.to_path_buf(),
        name: "test-tree".to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
    });
    crate::engine::state::write(&crate::domain::tree::state_path(&tree.path), state).unwrap();
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

fn fresh_state(name: &str, created_at: &str, last_synthesized_at: Option<&str>) -> TreeState {
    TreeState {
        tree: TreeMetadata {
            name: name.to_string(),
            created_at: Timestamp::parse(created_at).unwrap(),
            last_synthesized_at: last_synthesized_at.map(|s| Timestamp::parse(s).unwrap()),
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

fn loaded_leaf(slug: &str, title: &str) -> LoadedLeaf {
    LoadedLeaf {
        slug: slug.to_string(),
        filename: format!("leaf/{slug}.md"),
        title: title.to_string(),
        summary: None,
        body: format!("body of {}", title),
        collected_at: "2026-01-01T00:00:00Z".to_string(),
    }
}

/// Minimal valid full-synthesis response: one branch over the given leaf refs.
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

/// Minimal valid incremental response helper.
fn incremental_response(updated: &str, new: &str) -> String {
    format!(
        r#"{{"updated_branches":{},"new_branches":{}}}"#,
        updated, new
    )
}

/// Set up a tree on disk with 4 leaves, 1 existing branch, and fresh leaf files.
fn setup_incremental_tree(dir: &Path) -> (SeededConfig, TreeState, Vec<LoadedLeaf>) {
    let mut state = fresh_state("test", "2026-01-01T00:00:00Z", Some("2026-01-10T00:00:00Z"));
    state.leaves.push(leaf_record(
        "leaf-a",
        "leaf/leaf-a.md",
        "Leaf A",
        "2026-01-15T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-b",
        "leaf/leaf-b.md",
        "Leaf B",
        "2026-01-15T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-c",
        "leaf/leaf-c.md",
        "Leaf C",
        "2026-01-05T00:00:00Z",
    ));
    state.leaves.push(leaf_record(
        "leaf-d",
        "leaf/leaf-d.md",
        "Leaf D",
        "2026-01-05T00:00:00Z",
    ));
    state.branches.push(branch_record(
        "existing",
        "Existing Branch",
        &["leaf-c", "leaf-d"],
    ));

    write_leaf(dir, "leaf/leaf-a.md", "---\ntitle: Leaf A\n---\n\nbody a\n");
    write_leaf(dir, "leaf/leaf-b.md", "---\ntitle: Leaf B\n---\n\nbody b\n");
    write_leaf(dir, "leaf/leaf-c.md", "---\ntitle: Leaf C\n---\n\nbody c\n");
    write_leaf(dir, "leaf/leaf-d.md", "---\ntitle: Leaf D\n---\n\nbody d\n");

    std::fs::create_dir_all(dir.join("branch")).unwrap();
    std::fs::write(
        dir.join("branch/existing.md"),
        "---\ntitle: Existing Branch\n---\n\n# Existing Branch\n\nbody\n",
    )
    .unwrap();

    write_state(dir, &state);

    let cfg = seeded_config(dir);
    let loaded = vec![
        loaded_leaf("leaf-a", "Leaf A"),
        loaded_leaf("leaf-b", "Leaf B"),
        loaded_leaf("leaf-c", "Leaf C"),
        loaded_leaf("leaf-d", "Leaf D"),
    ];
    (cfg, state, loaded)
}

// ── schema derivation ────────────────────────────────────────────────────────

#[test]
fn derived_synthesis_schema_requires_branches() {
    let schema = serde_json::to_value(crate::engine::schema::inline_schema_for::<
        BranchSynthesisResponse,
    >())
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
fn derived_incremental_synthesis_schema_requires_updated_and_new_branches() {
    let schema = serde_json::to_value(crate::engine::schema::inline_schema_for::<
        IncrementalSynthesisResponse,
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

#[test]
fn derived_synthesis_schema_has_no_ref_or_defs_or_schema_key() {
    let schema = serde_json::to_value(crate::engine::schema::inline_schema_for::<
        BranchSynthesisResponse,
    >())
    .unwrap();
    let json_str = schema.to_string();
    assert!(!json_str.contains("\"$schema\""));
    assert!(!json_str.contains("\"definitions\""));
    assert!(!json_str.contains("\"$ref\""));
}

#[test]
fn derived_incremental_synthesis_schema_has_no_ref_or_defs_or_schema_key() {
    let schema = serde_json::to_value(crate::engine::schema::inline_schema_for::<
        IncrementalSynthesisResponse,
    >())
    .unwrap();
    let json_str = schema.to_string();
    assert!(!json_str.contains("\"$schema\""));
    assert!(!json_str.contains("\"definitions\""));
    assert!(!json_str.contains("\"$ref\""));
}

// ── leaf reference fidelity ───────────────────────────────────────────────

#[test]
fn valid_leaf_reference_map_resolves_by_filename_stem_and_unique_title() {
    let leaves = vec![
        loaded_leaf("alpha-concept", "Alpha Concept"),
        loaded_leaf("beta-thing", "Beta Thing"),
    ];
    let lookup = leaf_resolver(&leaves);

    assert!(lookup.collisions.is_empty());
    assert_eq!(
        lookup.map.get("leaf/alpha-concept.md"),
        Some(&"leaf/alpha-concept.md".to_string())
    );
    assert_eq!(
        lookup.map.get("leaf/alpha-concept"),
        Some(&"leaf/alpha-concept.md".to_string())
    );
    assert_eq!(
        lookup.map.get("alpha-concept"),
        Some(&"leaf/alpha-concept.md".to_string())
    );
    assert_eq!(
        lookup.map.get("alpha concept"),
        Some(&"leaf/alpha-concept.md".to_string())
    );
    assert_eq!(
        lookup.map.get("leaf/beta-thing.md"),
        Some(&"leaf/beta-thing.md".to_string())
    );
}

#[test]
fn valid_leaf_reference_map_drops_ambiguous_title_keys() {
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
    assert_eq!(
        lookup.map.get("gamma-one"),
        Some(&"leaf/gamma-one.md".to_string())
    );
    assert_eq!(
        lookup.map.get("gamma-two"),
        Some(&"leaf/gamma-two.md".to_string())
    );
}

#[test]
fn collision_warnings_captured_as_data_not_printed() {
    let leaves = vec![
        loaded_leaf("gamma-one", "Shared Topic"),
        loaded_leaf("gamma-two", "Shared Topic"),
    ];
    let mut warnings = Vec::new();
    parse_and_validate_with_input_size(
        &branch_response(&["gamma-one", "leaf/gamma-two.md"]),
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

    let plan = parse_and_validate_with_input_size(
        &branch_response(&["alpha-concept", "leaf/beta-thing.md"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .expect("refs by slug and filename must resolve");
    assert_eq!(
        plan.branches[0].leaves,
        vec!["leaf/alpha-concept.md", "leaf/beta-thing.md"]
    );

    parse_and_validate_with_input_size(
        &branch_response(&["Alpha Concept", "Beta Thing"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .expect("refs by unique title must resolve");
}

#[test]
fn parse_resolves_disambiguated_leaf_slug() {
    let leaves = vec![
        loaded_leaf("shared-topic-a1b2c3", "Shared Topic"),
        loaded_leaf("other-topic", "Other Topic"),
    ];

    let plan = parse_and_validate_with_input_size(
        &branch_response(&["shared-topic-a1b2c3", "other-topic"]),
        &leaves,
        1024,
        &mut Vec::new(),
    )
    .expect("exact state slugs must resolve independently of titles");

    assert_eq!(
        plan.branches[0].leaves,
        vec!["leaf/shared-topic-a1b2c3.md", "leaf/other-topic.md"]
    );
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
        matches!(err, SynthesisError::Validation(ref msg) if msg.contains("unknown leaf")),
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
        matches!(err, SynthesisError::Validation(ref msg) if msg.contains("unknown leaf")),
        "ambiguous title reference must fail validation, not silently resolve: {:?}",
        err
    );
}

// ── full-mode parse validation gaps ──────────────────────────────────────

#[test]
fn parse_full_rejects_empty_title() {
    let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
    let json = r#"{"branches":[{"title":"","body":"some body","leaves":["a","b"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, SynthesisError::Validation(ref msg) if msg.contains("empty title")));
}

#[test]
fn parse_full_rejects_empty_body() {
    let leaves = vec![loaded_leaf("a", "A"), loaded_leaf("b", "B")];
    let json = r#"{"branches":[{"title":"Concept","body":"","leaves":["a","b"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(matches!(err, SynthesisError::Validation(ref msg) if msg.contains("empty body")));
}

#[test]
fn parse_full_rejects_duplicate_slug() {
    let leaves = vec![
        loaded_leaf("a", "A"),
        loaded_leaf("b", "B"),
        loaded_leaf("c", "C"),
        loaded_leaf("d", "D"),
    ];
    let json = r#"{"branches":[{"title":"Same Thing","body":"body","leaves":["a","b"]},{"title":"Same Thing","body":"body","leaves":["c","d"]}]}"#;
    let err = parse_and_validate_with_input_size(json, &leaves, 1024, &mut Vec::new()).unwrap_err();
    assert!(
        matches!(err, SynthesisError::Validation(ref msg) if msg.contains("duplicate branch slug"))
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
    assert!(
        matches!(err, SynthesisError::Validation(ref msg) if msg.contains("at least 2 leaves"))
    );
}

// ── incremental-mode parse validation ─────────────────────────────────────

#[test]
fn parse_incremental_update_preserves_existing_leaves_and_adds_new() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

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
    assert!(plan.branches[0]
        .leaves
        .contains(&"leaf/leaf-a.md".to_string()));
    assert!(plan.branches[0]
        .leaves
        .contains(&"leaf/leaf-c.md".to_string()));
    assert!(plan.branches[0]
        .leaves
        .contains(&"leaf/leaf-d.md".to_string()));
}

#[test]
fn parse_incremental_update_dropping_existing_leaf_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

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
        matches!(err, SynthesisError::Validation(ref msg) if msg.contains("dropped existing leaf"))
    );
}

#[test]
fn parse_incremental_new_branch_without_new_leaf_is_dropped() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

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
    assert!(matches!(err, SynthesisError::Validation(ref msg) if msg.contains("unknown branch")));
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
    assert!(matches!(err, SynthesisError::Validation(ref msg) if msg.contains("changed title")));
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
    assert!(matches!(err, SynthesisError::Validation(ref msg) if msg.contains("empty title")));
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
    assert!(matches!(err, SynthesisError::Validation(ref msg) if msg.contains("empty body")));
}

#[test]
fn parse_incremental_update_with_no_new_leaf_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

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
        matches!(err, SynthesisError::Validation(ref msg) if msg.contains("no newly processed leaf"))
    );
}

#[test]
fn parse_incremental_insufficient_leaves_errors() {
    let dir = TempDir::new().unwrap();
    let (cfg, _, loaded) = setup_incremental_tree(dir.path());

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
    assert!(
        matches!(err, SynthesisError::Validation(ref msg) if msg.contains("at least 2 leaves"))
    );
}
