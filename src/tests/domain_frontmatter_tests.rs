use super::*;
use crate::domain::{Timestamp, Title, Url};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

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

// ── flatten probe: serde_yaml_ng flatten + round-trip ───────────────────

/// Read-side frontmatter struct with typed known fields and flattened extras.
/// This probes serde_yaml_ng's `#[serde(flatten)]` support for the Phase 2 GO/NO-GO.
#[derive(Debug, Deserialize, Serialize)]
struct ReadLeafFrontmatter {
    #[serde(default)]
    title: Option<Title>,
    #[serde(default)]
    url: Option<Url>,
    #[serde(default)]
    collected_at: Option<Timestamp>,
    #[serde(flatten)]
    extra: Mapping,
}

#[test]
fn flatten_probe_typed_fields_deserialized_correctly() {
    let yaml = "\
title: My Leaf\nurl: https://example.com\ncollected_at: 2025-01-15T09:32:00.000Z\ncount: 42\nflag: true\ntags:\n  - rust\n  - cli\nnote: extra string\n";
    let fm: ReadLeafFrontmatter = serde_yaml_ng::from_str(yaml).unwrap();

    assert_eq!(fm.title.as_ref().map(|t| t.as_str()), Some("My Leaf"));
    assert_eq!(
        fm.url.as_ref().map(|u| u.as_str()),
        Some("https://example.com")
    );
    assert!(fm.collected_at.is_some());
    assert_eq!(fm.extra.get("count").and_then(Value::as_i64), Some(42));
    assert_eq!(fm.extra.get("flag").and_then(Value::as_bool), Some(true));
    assert_eq!(
        fm.extra.get("note").and_then(Value::as_str),
        Some("extra string")
    );
    // tags sequence
    let tags = fm.extra.get("tags").and_then(Value::as_sequence);
    assert!(tags.is_some());
    let tags: Vec<&str> = tags.unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(tags, vec!["rust", "cli"]);
}

#[test]
fn flatten_probe_round_trips_extra_keys_without_loss() {
    let yaml = "\
title: Test\nurl: https://a.com\ncollected_at: 2025-01-01T00:00:00.000Z\nint_val: 7\nbool_val: false\nnull_val: null\nseq_val:\n  - a\n  - b\nstr_val: hello\n";
    let fm: ReadLeafFrontmatter = serde_yaml_ng::from_str(yaml).unwrap();

    // Re-serialize and parse again
    let roundtripped = serde_yaml_ng::to_string(&fm).unwrap();
    let fm2: ReadLeafFrontmatter = serde_yaml_ng::from_str(&roundtripped).unwrap();

    // Typed fields preserved
    assert_eq!(fm2.title.as_ref().map(|t| t.as_str()), Some("Test"));
    assert_eq!(fm2.url.as_ref().map(|u| u.as_str()), Some("https://a.com"));
    assert!(fm2.collected_at.is_some());

    // Extra keys preserved
    assert_eq!(fm2.extra.get("int_val").and_then(Value::as_i64), Some(7));
    assert_eq!(
        fm2.extra.get("bool_val").and_then(Value::as_bool),
        Some(false)
    );
    assert!(fm2
        .extra
        .get("null_val")
        .map(|v| v.is_null())
        .unwrap_or(false));
    assert_eq!(
        fm2.extra.get("str_val").and_then(Value::as_str),
        Some("hello")
    );
    let seq = fm2
        .extra
        .get("seq_val")
        .and_then(Value::as_sequence)
        .unwrap();
    assert_eq!(seq.len(), 2);
    assert_eq!(seq[0].as_str(), Some("a"));
    assert_eq!(seq[1].as_str(), Some("b"));
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
