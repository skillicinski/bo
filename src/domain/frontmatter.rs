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

#[cfg(test)]
#[path = "../tests/domain_frontmatter_tests.rs"]
mod tests;
