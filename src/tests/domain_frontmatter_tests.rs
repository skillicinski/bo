use super::*;
use serde_yaml_ng::Value;

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
    m.insert(
        Value::String("title".into()),
        Value::String("My Branch".into()),
    );
    m.insert(
        Value::String("compiled_at".into()),
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
    m.insert(Value::String("title".into()), Value::String("Test".into()));
    m.insert(
        Value::String("compiled_at".into()),
        Value::String("2025-01-01T00:00:00Z".into()),
    );
    m.insert(
        Value::String("leaves".into()),
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
    m.insert(Value::String("count".into()), Value::Number(42.into()));
    m.insert(Value::String("flag".into()), Value::Bool(true));
    m.insert(Value::String("null_val".into()), Value::Null);
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
