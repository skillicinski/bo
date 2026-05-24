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

    let doc = render(&m, "# My Branch\n\nBody.\n").unwrap();
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

    let doc = render(&m, "# Test\n\nBody.\n").unwrap();
    let (parsed_m, body) = parse(&doc).unwrap();
    assert_eq!(parsed_m.get("title").and_then(|v| v.as_str()), Some("Test"));
    assert!(body.contains("Body."));
}

#[test]
fn render_returns_ok_for_standard_mapping() {
    // Standard Mapping values should always serialize successfully
    // and round-trip through parse — no silent empty YAML on failure.
    let mut m = Mapping::new();
    set_field(&mut m, "count", Value::Number(42.into()));
    set_field(&mut m, "flag", Value::Bool(true));
    set_field(&mut m, "null_val", Value::Null);
    let result = render(&m, "body");
    assert!(result.is_ok());
    let doc = result.unwrap();
    let (parsed_m, body) = parse(&doc).unwrap();
    assert_eq!(parsed_m.get("count").and_then(|v| v.as_i64()), Some(42));
    assert_eq!(parsed_m.get("flag").and_then(|v| v.as_bool()), Some(true));
    assert!(parsed_m
        .get("null_val")
        .map(|v| v.is_null())
        .unwrap_or(false));
    assert!(body.contains("body"));
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
