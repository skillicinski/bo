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
) -> io::Result<()> {
    let content = format_content(title, url, collected_at, body);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

/// Format leaf document content — frontmatter block followed by body.
///
/// Kept private; callers should use `write`. Separated only so that the
/// formatting logic can be exercised in tests without touching the filesystem.
fn format_content(title: Option<&str>, url: &str, collected_at: &str, body: &str) -> String {
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

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // ── write ─────────────────────────────────────────────────────────────────

    #[test]
    fn write_creates_file_with_all_fields() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("my-article.md");

        write(
            &path,
            Some("My Article"),
            "https://example.com",
            "2025-01-15T09:32:00Z",
            "Some content here.",
        )
        .unwrap();

        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.starts_with("---\n"));
        assert!(content.contains("title: \"My Article\""));
        assert!(content.contains("url: https://example.com"));
        assert!(content.contains("collected_at: 2025-01-15T09:32:00Z"));
        assert!(content.contains("updated_at: 2025-01-15T09:32:00Z"));
        assert!(content.contains("# My Article"));
        assert!(content.contains("Some content here."));
    }

    #[test]
    fn write_with_no_title_omits_heading() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("no-title.md");

        write(
            &path,
            None,
            "https://example.com",
            "2025-01-15T09:32:00Z",
            "Body only.",
        )
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("title: \"\""));
        assert!(!content.contains("# ")); // no heading
    }

    #[test]
    fn write_escapes_yaml_special_chars_in_title() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("special.md");

        write(
            &path,
            Some("Rust: A \"Fast\" Language"),
            "https://example.com",
            "2025-01-15T09:32:00Z",
            "Content.",
        )
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains(r#"title: "Rust: A \"Fast\" Language""#));
    }

    #[test]
    fn write_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sub").join("dir").join("article.md");

        write(
            &path,
            None,
            "https://example.com",
            "2025-01-01T00:00:00Z",
            "content",
        )
        .unwrap();

        assert!(path.exists());
    }

    #[test]
    fn write_appends_trailing_newline_if_body_lacks_one() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("article.md");

        write(
            &path,
            None,
            "https://example.com",
            "2025-01-01T00:00:00Z",
            "no newline",
        )
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.ends_with('\n'));
    }

    #[test]
    fn write_does_not_double_newline_if_body_already_ends_with_one() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("article.md");

        write(
            &path,
            None,
            "https://example.com",
            "2025-01-01T00:00:00Z",
            "has newline\n",
        )
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.ends_with("\n\n"));
    }

    // ── read_frontmatter ──────────────────────────────────────────────────────

    #[test]
    fn read_frontmatter_returns_mapping_for_valid_leaf() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("article.md");

        write(
            &path,
            Some("Test Article"),
            "https://example.com",
            "2025-06-01T10:00:00Z",
            "Body.\n",
        )
        .unwrap();

        let mapping = read_frontmatter(&path).unwrap();
        assert_eq!(
            mapping.get("title").and_then(|v| v.as_str()),
            Some("Test Article")
        );
        assert_eq!(
            mapping.get("url").and_then(|v| v.as_str()),
            Some("https://example.com")
        );
    }

    #[test]
    fn read_frontmatter_returns_io_error_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.md");
        let err = read_frontmatter(&path).unwrap_err();
        assert!(matches!(err, LeafError::Io(_)));
    }

    #[test]
    fn read_frontmatter_returns_frontmatter_error_for_bad_yaml() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("bad.md");
        fs::write(&path, "---\n: invalid: yaml\n---\n\nbody\n").unwrap();
        let err = read_frontmatter(&path).unwrap_err();
        assert!(matches!(err, LeafError::Frontmatter(_)));
    }

    #[test]
    fn read_frontmatter_returns_missing_error_for_no_delimiters() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("plain.md");
        fs::write(&path, "# Just markdown\n\nNo frontmatter.\n").unwrap();
        let err = read_frontmatter(&path).unwrap_err();
        assert!(matches!(
            err,
            LeafError::Frontmatter(FrontmatterError::Missing)
        ));
    }
}
