// YAML frontmatter parsing and rendering.

use serde_yaml_ng::{Mapping, Value};
use std::fmt;

// ── errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum FrontmatterError {
    /// No `---` delimiters found.
    Missing,
    /// YAML inside the delimiters could not be parsed.
    Parse(String),
    /// YAML serialization failed (should not occur for standard Mapping values).
    Serialization(String),
}

impl fmt::Display for FrontmatterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrontmatterError::Missing => write!(f, "no frontmatter delimiters found"),
            FrontmatterError::Parse(msg) => write!(f, "invalid YAML frontmatter: {}", msg),
            FrontmatterError::Serialization(msg) => {
                write!(f, "YAML serialization error: {}", msg)
            }
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
pub fn render(mapping: &Mapping, body: &str) -> Result<String, FrontmatterError> {
    let yaml = serde_yaml_ng::to_string(mapping)
        .map_err(|e| FrontmatterError::Serialization(e.to_string()))?;
    Ok(format!("---\n{}---\n\n{}", yaml, body))
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
