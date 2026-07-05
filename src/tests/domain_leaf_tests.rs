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

fn title(s: &str) -> Title {
    Title::parse(s).unwrap()
}

// ── format_content title escaping tests ───────────────────────────────────

#[test]
fn format_content_title_with_newlines_is_valid_yaml() {
    let content = format_content(
        Some(&title("Line 1\nLine 2\nLine 3")),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "Body.",
    );
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some("Line 1\nLine 2\nLine 3")
    );
}

#[test]
fn format_content_title_with_tabs_is_valid_yaml() {
    let content = format_content(
        Some(&title("Col1\tCol2")),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "Body.",
    );
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some("Col1\tCol2")
    );
}

#[test]
fn format_content_title_with_cr_is_valid_yaml() {
    let content = format_content(
        Some(&title("With\rCarriage")),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "Body.",
    );
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some("With\rCarriage")
    );
}

#[test]
fn format_content_title_with_backslash_and_quote_is_valid_yaml() {
    let content = format_content(
        Some(&title(r#"Path \to\file and "quoted""#)),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "Body.",
    );
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some(r#"Path \to\file and "quoted""#)
    );
}

// ── omitted fields ────────────────────────────────────────────────────────

#[test]
fn format_content_omits_summary_and_updated_at() {
    let content = format_content(
        Some(&title("Article")),
        &url("https://example.com"),
        &ts("2025-01-01T00:00:00Z"),
        "Body.",
    );
    let (mapping, _) = frontmatter::parse(&content).unwrap();
    assert!(mapping.get("summary").is_none());
    assert!(mapping.get("updated_at").is_none());
    assert!(mapping.get("title").is_some());
    assert!(mapping.get("url").is_some());
    assert!(mapping.get("collected_at").is_some());
}

// ── Leaf serde: title migration ──────────────────────────────────────────

fn leaf_json(title: &str) -> String {
    format!(
        r#"{{
            "slug": "test-leaf",
            "file": "test-leaf.md",
            "title": {title},
            "url": "https://example.com/test",
            "collected_at": "2025-01-01T00:00:00.000Z",
            "summary": null
        }}"#
    )
}

#[test]
fn leaf_deserialize_empty_title_is_none() {
    let json = leaf_json("\"\"");
    let leaf: Leaf = serde_json::from_str(&json).unwrap();
    assert!(leaf.title.is_none());
}

#[test]
fn leaf_deserialize_missing_title_is_none() {
    let json = r#"{
        "slug": "test-leaf",
        "file": "test-leaf.md",
        "url": "https://example.com/test",
        "collected_at": "2025-01-01T00:00:00.000Z",
        "summary": null
    }"#;
    let leaf: Leaf = serde_json::from_str(json).unwrap();
    assert!(leaf.title.is_none());
}

#[test]
fn leaf_deserialize_valid_title_is_some() {
    let json = leaf_json("\"X\"");
    let leaf: Leaf = serde_json::from_str(&json).unwrap();
    assert_eq!(leaf.title.as_ref().map(|t| t.as_str()), Some("X"));
}

#[test]
fn leaf_with_none_title_serializes_as_null() {
    let leaf = Leaf {
        slug: crate::domain::Slug::parse("test-leaf").unwrap(),
        file: "test-leaf.md".to_string(),
        title: None,
        url: Url::parse("https://example.com/test").unwrap(),
        collected_at: Timestamp::parse("2025-01-01T00:00:00.000Z").unwrap(),
        summary: None,
    };
    let json = serde_json::to_string_pretty(&leaf).unwrap();
    assert!(json.contains("\"title\": null"));
}
