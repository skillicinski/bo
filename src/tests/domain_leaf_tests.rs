use super::*;
use crate::domain::frontmatter;
use crate::domain::{Timestamp, Title, Url};

// ── helpers ───────────────────────────────────────────────────────────────

fn ts(s: &str) -> Timestamp {
    Timestamp::parse(s).unwrap()
}

fn url(s: &str) -> Url {
    Url::parse(s).unwrap()
}

// ── format_content title escaping tests ───────────────────────────────────

#[test]
fn format_content_title_with_newlines_is_valid_yaml() {
    let title = Title::new("Line 1\nLine 2\nLine 3");
    let content = format_content(
        Some(&title),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "Body.",
        None,
    );
    // Should contain escaped newlines in YAML
    assert!(content.contains("\\n"));
    // Content must be parseable as YAML frontmatter
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some("Line 1\nLine 2\nLine 3")
    );
}

#[test]
fn format_content_title_with_tabs_is_valid_yaml() {
    let title = Title::new("Col1\tCol2");
    let content = format_content(
        Some(&title),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "Body.",
        None,
    );
    assert!(content.contains("\\t"));
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some("Col1\tCol2")
    );
}

#[test]
fn format_content_title_with_cr_is_valid_yaml() {
    let title = Title::new("With\rCarriage");
    let content = format_content(
        Some(&title),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "Body.",
        None,
    );
    assert!(content.contains("\\r"));
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some("With\rCarriage")
    );
}

#[test]
fn format_content_title_with_backslash_and_quote_is_valid_yaml() {
    let title = Title::new(r#"Path \to\file and "quoted""#);
    let content = format_content(
        Some(&title),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "Body.",
        None,
    );
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some(r#"Path \to\file and "quoted""#)
    );
}

// ── summary tests ─────────────────────────────────────────────────────────

#[test]
fn format_content_with_single_line_summary() {
    let content = format_content(
        Some(&Title::new("Article")),
        &url("https://example.com"),
        &ts("2025-01-01T00:00:00Z"),
        "Body content.",
        Some("This is a single-line summary of the article."),
    );
    assert!(content.contains("summary: \"This is a single-line summary of the article.\""));
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("summary").and_then(|v| v.as_str()),
        Some("This is a single-line summary of the article.")
    );
}

#[test]
fn format_content_with_multi_line_summary() {
    let summary = "First line of the summary.\nSecond line continues.\nThird line ends.";
    let content = format_content(
        Some(&Title::new("Article")),
        &url("https://example.com"),
        &ts("2025-01-01T00:00:00Z"),
        "Body.",
        Some(summary),
    );
    assert!(content.contains("summary: |\n"));
    assert!(content.contains("  First line of the summary.\n"));
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    let parsed = mapping.get("summary").and_then(|v| v.as_str()).unwrap();
    assert!(parsed.contains("First line"));
    assert!(parsed.contains("Third line"));
}

#[test]
fn format_content_with_summary_containing_special_chars() {
    let summary = "Rust's \"ownership\" model: memory safety without GC.";
    let content = format_content(
        Some(&Title::new("Article")),
        &url("https://example.com"),
        &ts("2025-01-01T00:00:00Z"),
        "Body.",
        Some(summary),
    );
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    let parsed = mapping.get("summary").and_then(|v| v.as_str()).unwrap();
    assert!(parsed.contains("ownership"));
    assert!(parsed.contains("Rust's"));
}

#[test]
fn format_content_with_none_summary_omits_field() {
    let content = format_content(
        Some(&Title::new("Article")),
        &url("https://example.com"),
        &ts("2025-01-01T00:00:00Z"),
        "Body.",
        None,
    );
    assert!(!content.contains("summary"));
}
