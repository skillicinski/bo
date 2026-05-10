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
        None,
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
        None,
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
        None,
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
        None,
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
        None,
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
        None,
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
        None,
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

// ── summary tests ─────────────────────────────────────────────────────────

#[test]
fn write_with_single_line_summary() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("summary.md");

    write(
        &path,
        Some("Article"),
        "https://example.com",
        "2025-01-01T00:00:00Z",
        "Body content.",
        Some("This is a single-line summary of the article."),
    )
    .unwrap();

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("summary: \"This is a single-line summary of the article.\""));

    let mapping = read_frontmatter(&path).unwrap();
    assert_eq!(
        mapping.get("summary").and_then(|v| v.as_str()),
        Some("This is a single-line summary of the article.")
    );
}

#[test]
fn write_with_multi_line_summary() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("multi.md");

    let summary = "First line of the summary.\nSecond line continues.\nThird line ends.";
    write(
        &path,
        Some("Article"),
        "https://example.com",
        "2025-01-01T00:00:00Z",
        "Body.",
        Some(summary),
    )
    .unwrap();

    let content = fs::read_to_string(&path).unwrap();
    assert!(content.contains("summary: |\n"));
    assert!(content.contains("  First line of the summary.\n"));

    let mapping = read_frontmatter(&path).unwrap();
    let parsed = mapping.get("summary").and_then(|v| v.as_str()).unwrap();
    assert!(parsed.contains("First line"));
    assert!(parsed.contains("Third line"));
}

#[test]
fn write_with_summary_containing_special_chars() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("special.md");

    let summary = "Rust's \"ownership\" model: memory safety without GC.";
    write(
        &path,
        Some("Article"),
        "https://example.com",
        "2025-01-01T00:00:00Z",
        "Body.",
        Some(summary),
    )
    .unwrap();

    let mapping = read_frontmatter(&path).unwrap();
    let parsed = mapping.get("summary").and_then(|v| v.as_str()).unwrap();
    assert!(parsed.contains("ownership"));
    assert!(parsed.contains("Rust's"));
}

#[test]
fn write_with_none_summary_omits_field() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nosummary.md");

    write(
        &path,
        Some("Article"),
        "https://example.com",
        "2025-01-01T00:00:00Z",
        "Body.",
        None,
    )
    .unwrap();

    let content = fs::read_to_string(&path).unwrap();
    assert!(!content.contains("summary"));
}
