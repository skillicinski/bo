// Branch file I/O.
//
// A branch is a synthesised concept file written by `bo compile`.
// It lives at {output_dir}/branches/{slug}.md and has YAML frontmatter
// followed by a markdown body beginning with a heading matching the title.

use crate::domain::frontmatter;
use serde_yaml_ng::{Mapping, Value};
use std::fs;
use std::io;
use std::path::Path;

/// Read the `compiled_at` value from an existing branch file.
///
/// Returns `None` in all failure cases: file absent, I/O error, unparseable
/// frontmatter, or missing `compiled_at` field.  The caller treats all of
/// these identically (first-write semantics).
pub fn read_compiled_at(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let (mapping, _) = frontmatter::parse(&content).ok()?;
    mapping
        .get("compiled_at")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Write a complete branch markdown file.
///
/// If `body` does not already begin with `# {title}`, the heading is
/// prepended automatically so the file always starts with the correct heading.
///
/// Parent directories are created as needed.
pub fn write(
    path: &Path,
    title: &str,
    body: &str,
    leaves: &[String],
    compiled_at: &str,
    updated_at: &str,
) -> io::Result<()> {
    // Build frontmatter mapping
    let mut mapping = Mapping::new();
    frontmatter::set_field(&mut mapping, "title", Value::String(title.to_string()));
    frontmatter::set_field(
        &mut mapping,
        "compiled_at",
        Value::String(compiled_at.to_string()),
    );
    frontmatter::set_field(
        &mut mapping,
        "updated_at",
        Value::String(updated_at.to_string()),
    );

    let leaves_seq = Value::Sequence(leaves.iter().map(|l| Value::String(l.clone())).collect());
    frontmatter::set_field(&mut mapping, "leaves", leaves_seq);

    // Ensure body starts with the correct heading
    let expected_heading = format!("# {}", title);
    let full_body = if body.starts_with(&expected_heading) {
        body.to_string()
    } else {
        format!("{}\n\n{}", expected_heading, body)
    };

    let content = frontmatter::render(&mapping, &full_body);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write_test_branch(dir: &TempDir, slug: &str, compiled_at: &str, updated_at: &str) {
        let path = dir.path().join(format!("{}.md", slug));
        write(
            &path,
            "Test Concept",
            "# Test Concept\n\nSome body.\n",
            &["leaf-a.md".to_string(), "leaf-b.md".to_string()],
            compiled_at,
            updated_at,
        )
        .unwrap();
    }

    #[test]
    fn write_creates_file_with_valid_frontmatter() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test-concept.md");

        write(
            &path,
            "Test Concept",
            "Some body.\n",
            &["leaf-a.md".to_string()],
            "2025-06-01T12:00:00Z",
            "2025-06-01T12:00:00Z",
        )
        .unwrap();

        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        let (mapping, _) = frontmatter::parse(&content).unwrap();
        assert_eq!(
            mapping.get("title").and_then(|v| v.as_str()),
            Some("Test Concept")
        );
        assert_eq!(
            mapping.get("compiled_at").and_then(|v| v.as_str()),
            Some("2025-06-01T12:00:00Z")
        );
        assert_eq!(
            mapping.get("updated_at").and_then(|v| v.as_str()),
            Some("2025-06-01T12:00:00Z")
        );
        let leaves = mapping.get("leaves").and_then(|v| v.as_sequence()).unwrap();
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].as_str(), Some("leaf-a.md"));
    }

    #[test]
    fn write_creates_parent_directories() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("branches").join("test-concept.md");
        assert!(!path.parent().unwrap().exists());

        write(
            &path,
            "T",
            "body\n",
            &[],
            "2025-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();
        assert!(path.exists());
    }

    #[test]
    fn write_prepends_heading_if_missing() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("branch.md");

        write(
            &path,
            "My Concept",
            "Body without heading.\n",
            &[],
            "2025-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let (_, body) = frontmatter::parse(&content).unwrap();
        assert!(body.starts_with("# My Concept"));
    }

    #[test]
    fn write_does_not_duplicate_heading_if_present() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("branch.md");

        write(
            &path,
            "My Concept",
            "# My Concept\n\nBody.\n",
            &[],
            "2025-01-01T00:00:00Z",
            "2025-01-01T00:00:00Z",
        )
        .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let heading_count = content.matches("# My Concept").count();
        assert_eq!(heading_count, 1);
    }

    #[test]
    fn read_compiled_at_returns_none_for_missing_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.md");
        assert!(read_compiled_at(&path).is_none());
    }

    #[test]
    fn read_compiled_at_returns_value_from_existing_file() {
        let dir = TempDir::new().unwrap();
        write_test_branch(
            &dir,
            "concept",
            "2025-06-01T12:00:00Z",
            "2025-06-01T12:00:00Z",
        );
        let path = dir.path().join("concept.md");
        assert_eq!(
            read_compiled_at(&path).as_deref(),
            Some("2025-06-01T12:00:00Z")
        );
    }

    #[test]
    fn second_write_preserves_compiled_at() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("concept.md");

        // First write
        write(
            &path,
            "Concept",
            "body\n",
            &[],
            "2025-06-01T12:00:00Z",
            "2025-06-01T12:00:00Z",
        )
        .unwrap();

        let original_compiled_at = read_compiled_at(&path).unwrap();
        assert_eq!(original_compiled_at, "2025-06-01T12:00:00Z");

        // Second write — updated_at advances, compiled_at stays
        let existing_compiled_at = read_compiled_at(&path).unwrap_or_else(|| "now".to_string());
        write(
            &path,
            "Concept",
            "updated body\n",
            &[],
            &existing_compiled_at,
            "2025-12-01T10:00:00Z",
        )
        .unwrap();

        assert_eq!(
            read_compiled_at(&path).as_deref(),
            Some("2025-06-01T12:00:00Z")
        );
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("updated_at: 2025-12-01T10:00:00Z"));
    }
}
