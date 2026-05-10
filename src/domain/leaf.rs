// Domain entity I/O for leaf documents.
//
// A leaf is a collected document produced by `bo add`. It lives at
// {output_dir}/{slug}.md and has YAML frontmatter followed by a markdown body.
//
// Analogous to branch.rs; together they define the two entity types in bo's
// knowledge graph.
//
// The title field is always double-quoted in the written YAML so that special
// characters (colons, embedded quotes) are escaped consistently. This is the
// canonical on-disk format for leaf files and is preserved by patch_fields
// when bo compile updates the frontmatter later.

use crate::domain::frontmatter::{self, FrontmatterError};
use serde_yaml_ng::Mapping;
use std::{fmt, fs, io, path::Path};

// ── errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum LeafError {
    Io(io::Error),
    Frontmatter(FrontmatterError),
}

impl fmt::Display for LeafError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LeafError::Io(e) => write!(f, "I/O error: {}", e),
            LeafError::Frontmatter(e) => write!(f, "frontmatter error: {}", e),
        }
    }
}

// ── write ─────────────────────────────────────────────────────────────────────

/// Write a leaf document to `path` (full path, including `.md` extension).
///
/// `collected_at` is used for both `collected_at` and `updated_at` on initial
/// write; they diverge only when `bo compile` later patches the frontmatter.
///
/// Creates parent directories if needed.
pub fn write(
    path: &Path,
    title: Option<&str>,
    url: &str,
    collected_at: &str,
    body: &str,
    summary: Option<&str>,
) -> io::Result<()> {
    let content = format_content(title, url, collected_at, body, summary);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

/// Format leaf document content — frontmatter block followed by body.
///
/// Kept private; callers should use `write`. Separated only so that the
/// formatting logic can be exercised in tests without touching the filesystem.
fn format_content(
    title: Option<&str>,
    url: &str,
    collected_at: &str,
    body: &str,
    summary: Option<&str>,
) -> String {
    let title_yaml = match title {
        Some(t) => format!("\"{}\"", t.replace('\\', "\\\\").replace('"', "\\\"")),
        None => "\"\"".to_string(),
    };

    let mut doc = String::new();
    doc.push_str("---\n");
    doc.push_str(&format!("title: {}\n", title_yaml));
    doc.push_str(&format!("url: {}\n", url));
    doc.push_str(&format!("collected_at: {}\n", collected_at));
    doc.push_str(&format!("updated_at: {}\n", collected_at));

    if let Some(s) = summary {
        if !s.is_empty() {
            if s.contains('\n') {
                doc.push_str("summary: |\n");
                for line in s.lines() {
                    doc.push_str("  ");
                    doc.push_str(line);
                    doc.push('\n');
                }
            } else {
                let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
                doc.push_str(&format!("summary: \"{}\"\n", escaped));
            }
        }
    }

    doc.push_str("---\n\n");

    if let Some(t) = title {
        doc.push_str(&format!("# {}\n\n", t));
    }

    doc.push_str(body);
    if !body.ends_with('\n') {
        doc.push('\n');
    }

    doc
}

// ── read ──────────────────────────────────────────────────────────────────────

/// Read and parse the frontmatter of an existing leaf file.
///
/// Returns the parsed YAML mapping so callers can inspect any field.
/// Returns `LeafError` if the file cannot be read or its frontmatter is
/// absent or malformed.
///
/// Used by `bo compile` to validate leaves before the agent run — both I/O
/// failures (missing file) and parse failures (bad YAML) are surfaced through
/// the same error type so the caller can treat them uniformly as "skip".
pub fn read_frontmatter(path: &Path) -> Result<Mapping, LeafError> {
    let content = fs::read_to_string(path).map_err(LeafError::Io)?;
    let (mapping, _) = frontmatter::parse(&content).map_err(LeafError::Frontmatter)?;
    Ok(mapping)
}

#[cfg(test)]
#[path = "../tests/domain_leaf_tests.rs"]
mod tests;
