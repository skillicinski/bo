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

// ── goldens: byte-stable on-disk format ──────────────────────────────────

#[test]
fn golden_leaf_with_title() {
    let content = format_content(
        Some(&title("Example Title")),
        &url("https://example.com/page"),
        &ts("2025-01-15T09:32:00Z"),
        "Body text.",
    );
    assert_eq!(
        content,
        "---\ntitle: Example Title\nurl: https://example.com/page\ncollected_at: 2025-01-15T09:32:00.000Z\n---\n\n# Example Title\n\nBody text.\n"
    );
}

#[test]
fn golden_leaf_without_title() {
    let content = format_content(
        None,
        &url("https://example.com/page"),
        &ts("2025-01-15T09:32:00Z"),
        "Body text.",
    );
    assert_eq!(
        content,
        "---\ntitle: ''\nurl: https://example.com/page\ncollected_at: 2025-01-15T09:32:00.000Z\n---\n\nBody text.\n"
    );
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

// ── title-heading dedup (issue #161) ─────────────────────────────────────

#[test]
fn format_content_skips_prepend_when_body_has_matching_h1() {
    // Mirrors the byline-then-H1 shape trafilatura produces for articles like
    // Thorsten Ball's "How to Build an Agent": the article's own H1 survives in
    // the body but isn't the very first line.
    let content = format_content(
        Some(&title("How to Build an Agent")),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "Thorsten Ball//April 15, 2025\n\n# How to Build an Agent\n\nBody.",
    );
    assert_eq!(
        content.matches("# How to Build an Agent").count(),
        1,
        "title heading should appear exactly once, got: {content}",
    );
}

#[test]
fn format_content_prepends_when_body_h1_differs_from_title() {
    // A body heading that doesn't match the title is a real section, not a
    // duplicate — bo's title is still prepended.
    let content = format_content(
        Some(&title("The Real Title")),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "# A Different Section\n\nBody.",
    );
    assert!(content.contains("# The Real Title"));
    assert!(content.contains("# A Different Section"));
}

#[test]
fn format_content_skips_prepend_is_case_insensitive() {
    let content = format_content(
        Some(&title("how to build an agent")),
        &url("https://example.com"),
        &ts("2025-01-15T09:32:00Z"),
        "# How To Build An Agent\n\nBody.",
    );
    assert_eq!(
        content.matches("# How To Build An Agent").count()
            + content.matches("# how to build an agent").count(),
        1,
        "matching should be case-insensitive, got: {content}",
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
