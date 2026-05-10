use super::*;

const SIMPLE_DOC: &str = "\
---
title: Simple Title
url: https://example.com/article
collected_at: 2025-06-01T12:00:00Z
updated_at: 2025-06-01T12:00:00Z
---

# Simple Title

Body content here.
";

const DOC_WITH_BRANCHES: &str = "\
---
title: Article
url: https://example.com
collected_at: 2025-06-01T12:00:00Z
updated_at: 2025-06-01T12:00:00Z
branches:
  - rust-ownership
  - systems-programming
---

Body.
";

const DOC_WITH_INLINE_BRANCHES: &str = "\
---
title: Article
url: https://example.com
collected_at: 2025-06-01T12:00:00Z
updated_at: 2025-06-01T12:00:00Z
branches: []
---

Body.
";

// ── parse tests ───────────────────────────────────────────────────────────

#[test]
fn parse_returns_mapping_and_body() {
    let (mapping, body) = parse(SIMPLE_DOC).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some("Simple Title")
    );
    assert!(body.contains("Body content here."));
    assert!(!body.starts_with('\n')); // leading blank line stripped
}

#[test]
fn parse_missing_delimiters_returns_error() {
    let err = parse("no frontmatter here").unwrap_err();
    assert!(matches!(err, FrontmatterError::Missing));
}

#[test]
fn parse_invalid_yaml_returns_error() {
    let bad = "---\n: invalid: yaml: [\n---\n\nbody\n";
    let err = parse(bad).unwrap_err();
    assert!(matches!(err, FrontmatterError::Parse(_)));
}

// ── render tests ──────────────────────────────────────────────────────────

#[test]
fn render_produces_valid_document() {
    let mut m = Mapping::new();
    set_field(&mut m, "title", Value::String("My Branch".into()));
    set_field(
        &mut m,
        "compiled_at",
        Value::String("2025-01-01T00:00:00Z".into()),
    );

    let doc = render(&m, "# My Branch\n\nBody.\n");
    assert!(doc.starts_with("---\n"));
    assert!(doc.contains("title: My Branch"));
    assert!(doc.contains("---\n\n# My Branch"));
}

#[test]
fn render_round_trips_through_parse() {
    let mut m = Mapping::new();
    set_field(&mut m, "title", Value::String("Test".into()));
    set_field(
        &mut m,
        "compiled_at",
        Value::String("2025-01-01T00:00:00Z".into()),
    );
    set_field(
        &mut m,
        "leaves",
        Value::Sequence(vec![Value::String("a.md".into())]),
    );

    let doc = render(&m, "# Test\n\nBody.\n");
    let (parsed_m, body) = parse(&doc).unwrap();
    assert_eq!(parsed_m.get("title").and_then(|v| v.as_str()), Some("Test"));
    assert!(body.contains("Body."));
}

// ── set_field tests ───────────────────────────────────────────────────────

#[test]
fn set_field_appends_new_key() {
    let mut m = Mapping::new();
    set_field(&mut m, "a", Value::String("1".into()));
    set_field(&mut m, "b", Value::String("2".into()));
    assert_eq!(m.len(), 2);
    let keys: Vec<&str> = m.keys().filter_map(|k| k.as_str()).collect();
    assert_eq!(keys, vec!["a", "b"]);
}

#[test]
fn set_field_replaces_existing_key_in_place() {
    let mut m = Mapping::new();
    set_field(&mut m, "a", Value::String("old".into()));
    set_field(&mut m, "b", Value::String("keep".into()));
    set_field(&mut m, "a", Value::String("new".into()));
    // Position preserved: a is still first
    let keys: Vec<&str> = m.keys().filter_map(|k| k.as_str()).collect();
    assert_eq!(keys, vec!["a", "b"]);
    assert_eq!(m.get("a").and_then(|v| v.as_str()), Some("new"));
}

// ── patch_fields tests ────────────────────────────────────────────────────

#[test]
fn patch_fields_updates_str_field_in_place() {
    let result = patch_fields(SIMPLE_DOC, &[("updated_at", "2025-12-01T10:00:00Z")], &[]).unwrap();
    assert!(result.contains("updated_at: 2025-12-01T10:00:00Z"));
    // Other fields unchanged
    assert!(result.contains("title: Simple Title"));
    assert!(result.contains("url: https://example.com/article"));
    assert!(result.contains("collected_at: 2025-06-01T12:00:00Z"));
}

#[test]
fn patch_fields_appends_new_str_field() {
    let result = patch_fields(SIMPLE_DOC, &[("new_field", "hello")], &[]).unwrap();
    assert!(result.contains("new_field: hello"));
}

#[test]
fn patch_fields_replaces_sequence_block() {
    let result = patch_fields(
        DOC_WITH_BRANCHES,
        &[],
        &[("branches", &["new-branch".to_string()])],
    )
    .unwrap();
    assert!(result.contains("branches:\n  - new-branch"));
    assert!(!result.contains("rust-ownership"));
    assert!(!result.contains("systems-programming"));
}

#[test]
fn patch_fields_replaces_inline_empty_sequence() {
    let result = patch_fields(
        DOC_WITH_INLINE_BRANCHES,
        &[],
        &[("branches", &["new-branch".to_string()])],
    )
    .unwrap();
    assert!(result.contains("branches:\n  - new-branch"));
    assert!(!result.contains("branches: []"));
}

#[test]
fn patch_fields_appends_new_sequence_field() {
    let result =
        patch_fields(SIMPLE_DOC, &[], &[("branches", &["concept-a".to_string()])]).unwrap();
    assert!(result.contains("branches:\n  - concept-a"));
}

#[test]
fn patch_fields_writes_empty_sequence_as_inline() {
    let result = patch_fields(DOC_WITH_BRANCHES, &[], &[("branches", &[] as &[String])]).unwrap();
    assert!(result.contains("branches: []"));
}

#[test]
fn patch_fields_body_is_byte_identical() {
    let result = patch_fields(
        SIMPLE_DOC,
        &[("updated_at", "2025-12-01T10:00:00Z")],
        &[("branches", &["x".to_string()])],
    )
    .unwrap();

    let orig_body = SIMPLE_DOC.split("\n---\n\n").nth(1).unwrap();
    let new_body = result.split("\n---\n\n").nth(1).unwrap();
    assert_eq!(orig_body, new_body);
}

#[test]
fn patch_fields_title_and_url_preserved_exactly() {
    // Verify that fields NOT in str_fields/seq_fields are byte-identical.
    let result = patch_fields(
        SIMPLE_DOC,
        &[("updated_at", "2026-01-01T00:00:00Z")],
        &[("branches", &["some-branch".to_string()])],
    )
    .unwrap();
    assert!(result.contains("title: Simple Title"));
    assert!(result.contains("url: https://example.com/article"));
    assert!(result.contains("collected_at: 2025-06-01T12:00:00Z"));
}

#[test]
fn patch_fields_no_ops_returns_equivalent_doc() {
    // Empty patches should leave the document structurally equivalent
    // (body and all fields preserved — only trailing-whitespace differences possible).
    let result = patch_fields(SIMPLE_DOC, &[], &[]).unwrap();
    let orig_body = SIMPLE_DOC.split("\n---\n\n").nth(1).unwrap();
    let new_body = result.split("\n---\n\n").nth(1).unwrap();
    assert_eq!(orig_body, new_body);
}

#[test]
fn patch_fields_missing_delimiters_returns_error() {
    let err = patch_fields("no frontmatter", &[], &[]).unwrap_err();
    assert!(matches!(err, FrontmatterError::Missing));
}
