use super::*;
use crate::domain::{Slug, Timestamp, Title};
use tempfile::TempDir;

fn write_test_branch(dir: &TempDir, slug: &str, created_at: &str, updated_at: &str) {
    let path = dir.path().join(format!("{}.md", slug));
    let title = Title::new("Test Concept");
    let leaves = vec![
        Slug::parse("leaf-a").unwrap(),
        Slug::parse("leaf-b").unwrap(),
    ];
    let ca = Timestamp::parse(created_at).unwrap();
    let ua = Timestamp::parse(updated_at).unwrap();
    write(
        &path,
        &title,
        "# Test Concept\n\nSome body.\n",
        &leaves,
        &ca,
        &ua,
    )
    .unwrap();
}

#[test]
fn write_creates_file_with_valid_frontmatter() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("test-concept.md");
    let title = Title::new("Test Concept");
    let leaves = vec![Slug::parse("leaf-a").unwrap()];
    let ts = Timestamp::parse("2025-06-01T12:00:00Z").unwrap();

    write(&path, &title, "Some body.\n", &leaves, &ts, &ts).unwrap();

    assert!(path.exists());
    let content = fs::read_to_string(&path).unwrap();
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some("Test Concept")
    );
    assert_eq!(
        mapping.get("created_at").and_then(|v| v.as_str()),
        Some("2025-06-01T12:00:00.000Z")
    );
    assert_eq!(
        mapping.get("updated_at").and_then(|v| v.as_str()),
        Some("2025-06-01T12:00:00.000Z")
    );
    let leaves_seq = mapping.get("leaves").and_then(|v| v.as_sequence()).unwrap();
    assert_eq!(leaves_seq.len(), 1);
    assert_eq!(leaves_seq[0].as_str(), Some("leaf-a"));
}

#[test]
fn write_creates_parent_directories() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("branches").join("test-concept.md");
    assert!(!path.parent().unwrap().exists());
    let title = Title::new("T");
    let ts = Timestamp::parse("2025-01-01T00:00:00Z").unwrap();

    write(&path, &title, "body\n", &[], &ts, &ts).unwrap();
    assert!(path.exists());
}

#[test]
fn write_prepends_heading_if_missing() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("branch.md");
    let title = Title::new("My Concept");
    let ts = Timestamp::parse("2025-01-01T00:00:00Z").unwrap();

    write(&path, &title, "Body without heading.\n", &[], &ts, &ts).unwrap();

    let content = fs::read_to_string(&path).unwrap();
    let (_, body) = frontmatter::parse(&content).unwrap();
    assert!(body.starts_with("# My Concept"));
}

#[test]
fn write_does_not_duplicate_heading_if_present() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("branch.md");
    let title = Title::new("My Concept");
    let ts = Timestamp::parse("2025-01-01T00:00:00Z").unwrap();

    write(&path, &title, "# My Concept\n\nBody.\n", &[], &ts, &ts).unwrap();

    let content = fs::read_to_string(&path).unwrap();
    let heading_count = content.matches("# My Concept").count();
    assert_eq!(heading_count, 1);
}

#[test]
fn read_created_at_returns_none_for_missing_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nonexistent.md");
    assert!(read_created_at(&path).is_none());
}

#[test]
fn read_created_at_returns_value_from_existing_file() {
    let dir = TempDir::new().unwrap();
    write_test_branch(
        &dir,
        "concept",
        "2025-06-01T12:00:00Z",
        "2025-06-01T12:00:00Z",
    );
    let path = dir.path().join("concept.md");
    let created = read_created_at(&path).unwrap();
    assert_eq!(created.to_rfc3339_millis(), "2025-06-01T12:00:00.000Z");
}

#[test]
fn second_write_preserves_created_at() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("concept.md");
    let title = Title::new("Concept");
    let ts1 = Timestamp::parse("2025-06-01T12:00:00Z").unwrap();

    // First write
    write(&path, &title, "body\n", &[], &ts1, &ts1).unwrap();

    let original_created_at = read_created_at(&path).unwrap();
    assert_eq!(
        original_created_at.to_rfc3339_millis(),
        "2025-06-01T12:00:00.000Z"
    );

    // Second write — updated_at advances, created_at stays
    let existing_created_at = read_created_at(&path).unwrap();
    let ts2 = Timestamp::parse("2025-12-01T10:00:00Z").unwrap();
    write(
        &path,
        &title,
        "updated body\n",
        &[],
        &existing_created_at,
        &ts2,
    )
    .unwrap();

    let created = read_created_at(&path).unwrap();
    assert_eq!(created.to_rfc3339_millis(), "2025-06-01T12:00:00.000Z");
    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("updated_at: 2025-12-01T10:00:00.000Z"));
}
