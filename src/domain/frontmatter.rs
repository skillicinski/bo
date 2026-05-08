// YAML frontmatter parsing, rendering, and surgical patching.
//
// Two update paths exist by design:
//
//   render(mapping, body)          — assembles a document from a fresh Mapping.
//                                    Used for branch files (built from scratch).
//                                    serde_yaml_ng quoting is fine here because
//                                    there is no "original" to compare against.
//
//   patch_fields(content, ...)     — updates specific fields in an existing
//                                    document. All other fields — including their
//                                    original quoting style — are preserved
//                                    byte-for-byte.  Used for leaf frontmatter
//                                    updates so that bo compile doesn't dirty
//                                    the title/url/collected_at fields.
//
// Rationale: serde_yaml_ng re-serialises strings in the most compact form
// (e.g. `"Simple Title"` → bare `Simple Title`, `"Rust: X"` → `'Rust: X'`).
// Round-tripping a leaf through parse→render would change title quoting on
// every compile run.  patch_fields avoids this entirely.

use serde_yaml_ng::{Mapping, Value};
use std::fmt;

// ── errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum FrontmatterError {
    /// No `---` delimiters found.
    Missing,
    /// YAML inside the delimiters could not be parsed.
    Parse(String),
}

impl fmt::Display for FrontmatterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrontmatterError::Missing => write!(f, "no frontmatter delimiters found"),
            FrontmatterError::Parse(msg) => write!(f, "invalid YAML frontmatter: {}", msg),
        }
    }
}

// ── parse ─────────────────────────────────────────────────────────────────────

/// Split a document into its frontmatter Mapping and body string.
///
/// The body is everything after the closing `---` line, with the blank
/// separator line stripped (so `body` starts directly with content).
pub fn parse(content: &str) -> Result<(Mapping, String), FrontmatterError> {
    let (yaml_str, body) = split_yaml_and_body(content)?;
    let mapping: Mapping =
        serde_yaml_ng::from_str(yaml_str).map_err(|e| FrontmatterError::Parse(e.to_string()))?;
    Ok((mapping, body.to_string()))
}

// ── render ────────────────────────────────────────────────────────────────────

/// Assemble a complete document from a Mapping and a body string.
///
/// Used when creating brand-new files (branch files).  The body must NOT
/// include a leading blank line; `render` inserts the `---` separator and
/// the blank line itself.
pub fn render(mapping: &Mapping, body: &str) -> String {
    let yaml = serde_yaml_ng::to_string(mapping).unwrap_or_default();
    format!("---\n{}---\n\n{}", yaml, body)
}

// ── set_field ─────────────────────────────────────────────────────────────────

/// Upsert a field in a Mapping.
///
/// If the key already exists the value is replaced in-place (the `IndexMap`
/// preserves the original position).  If the key is absent it is appended.
pub fn set_field(mapping: &mut Mapping, key: &str, value: Value) {
    let k = Value::String(key.to_string());
    mapping.insert(k, value);
}

// ── patch_fields ──────────────────────────────────────────────────────────────

/// Surgically update specific fields in an existing document without touching
/// any other content.
///
/// * `str_fields` — scalar `key: value` pairs.  If the key already exists its
///   line is replaced; if absent it is appended.
/// * `seq_fields` — sequence fields.  Any existing `key:` block (key line plus
///   all subsequent indented continuation lines) is removed, and the new block
///   is appended at the end of the frontmatter.
///
/// The document body and the quoting style of all unmodified fields are
/// preserved byte-for-byte.
pub fn patch_fields(
    content: &str,
    str_fields: &[(&str, &str)],
    seq_fields: &[(&str, &[String])],
) -> Result<String, FrontmatterError> {
    // ── locate the YAML block ────────────────────────────────────────────────
    let without_open = content
        .strip_prefix("---\n")
        .ok_or(FrontmatterError::Missing)?;

    let close_pos = without_open
        .find("\n---")
        .ok_or(FrontmatterError::Missing)?;

    // yaml_str includes the trailing \n before the closing ---
    let yaml_str = &without_open[..close_pos + 1];
    // suffix is everything starting from \n--- (e.g. "\n---\n\nbody" or "\n---")
    let suffix = &without_open[close_pos..];

    // Validate YAML
    serde_yaml_ng::from_str::<Mapping>(yaml_str)
        .map_err(|e| FrontmatterError::Parse(e.to_string()))?;

    // ── work with YAML lines ─────────────────────────────────────────────────
    let mut lines: Vec<String> = yaml_str.lines().map(String::from).collect();

    // Handle str_fields: replace existing lines, track missing ones
    let mut str_found = vec![false; str_fields.len()];
    for line in &mut lines {
        for (idx, (key, value)) in str_fields.iter().enumerate() {
            let prefix = format!("{}:", key);
            if line == &prefix || line.starts_with(&format!("{}: ", key)) {
                *line = format!("{}: {}", key, value);
                str_found[idx] = true;
                break;
            }
        }
    }

    // Handle seq_fields: remove existing key blocks entirely
    let mut seq_found = vec![false; seq_fields.len()];
    for (idx, (key, _)) in seq_fields.iter().enumerate() {
        let key_line_bare = format!("{}:", key);
        let key_line_prefix = format!("{}: ", key);
        let mut i = 0;
        while i < lines.len() {
            if lines[i] == key_line_bare || lines[i].starts_with(&key_line_prefix) {
                seq_found[idx] = true;
                lines.remove(i);
                // Remove subsequent indented continuation lines
                while i < lines.len() && (lines[i].starts_with(' ') || lines[i].starts_with('\t')) {
                    lines.remove(i);
                }
                break;
            }
            i += 1;
        }
    }

    // ── rebuild the YAML string ──────────────────────────────────────────────
    let mut yaml = lines.join("\n");
    if !yaml.ends_with('\n') {
        yaml.push('\n');
    }

    // Append str_fields that were not found in the original
    for (idx, (key, value)) in str_fields.iter().enumerate() {
        if !str_found[idx] {
            yaml.push_str(&format!("{}: {}\n", key, value));
        }
    }

    // Always append seq_fields (existing blocks were removed above)
    for (key, values) in seq_fields.iter() {
        if values.is_empty() {
            yaml.push_str(&format!("{}: []\n", key));
        } else {
            yaml.push_str(&format!("{}:\n", key));
            for v in *values {
                yaml.push_str(&format!("  - {}\n", v));
            }
        }
    }

    // ── reassemble ───────────────────────────────────────────────────────────
    // Re-use the original suffix (preserves separator + body exactly)
    Ok(format!("---\n{}{}", yaml, suffix))
}

// ── internal helpers ──────────────────────────────────────────────────────────

/// Split content into (yaml_str, body) for `parse()`.
/// `body` has the leading blank line stripped.
fn split_yaml_and_body(content: &str) -> Result<(&str, &str), FrontmatterError> {
    let rest = content
        .strip_prefix("---\n")
        .ok_or(FrontmatterError::Missing)?;

    let close_pos = rest.find("\n---").ok_or(FrontmatterError::Missing)?;

    let yaml_str = &rest[..close_pos + 1];
    let after = &rest[close_pos + 5..]; // skip \n---\n
                                        // Strip optional blank separator line to get the body
    let body = after.strip_prefix('\n').unwrap_or(after);

    Ok((yaml_str, body))
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
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
        let result =
            patch_fields(SIMPLE_DOC, &[("updated_at", "2025-12-01T10:00:00Z")], &[]).unwrap();
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
        let result =
            patch_fields(DOC_WITH_BRANCHES, &[], &[("branches", &[] as &[String])]).unwrap();
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
}
