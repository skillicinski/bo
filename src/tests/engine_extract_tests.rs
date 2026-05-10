use super::*;

// ── scenario tests (public API) ──────────────────────────────────────────

const ARTICLE_WITH_LINKS: &str = r#"<html><head><title>Link Article</title></head>
<body><article>
<h1>Link Article</h1>
<p>Visit <a href="https://example.com">this website</a> for more details.
Also see <a href="https://other.com">another resource</a> for additional
information that helps with understanding the topic at hand.</p>
</article></body></html>"#;

const ARTICLE_WITHOUT_LINKS: &str = r#"<html><head><title>Plain Article</title></head>
<body><article>
<h1>Plain Article</h1>
<p>This article contains no hyperlinks at all. It provides substantial content
to pass the minimum extraction threshold for quality filtering purposes.</p>
</article></body></html>"#;

const ARTICLE_MATCHING_H1: &str = r#"<html><head><title>My Article</title></head>
<body><article>
<h1>My Article</h1>
<p>Body content that provides enough substance to pass the extraction quality
threshold. This is the main content paragraph of the article.</p>
<h2>A Section</h2>
<p>More content in this section for additional context and length.</p>
</article></body></html>"#;

const ARTICLE_DIFFERENT_H1: &str = r#"<html><head><title>Page Title</title></head>
<body><article>
<h1>Section Heading</h1>
<p>Content under a heading that differs from the page title. This provides
enough text to meet the minimum extraction threshold for quality filtering.</p>
</article></body></html>"#;

#[test]
fn links_in_article_body_are_stripped_to_plain_text() {
    let result = extract_content(ARTICLE_WITH_LINKS).unwrap();
    assert!(
        !result.body_markdown.contains("]("),
        "body_markdown should not contain markdown links, got: {}",
        result.body_markdown
    );
    // Anchor text is preserved as plain text
    assert!(
        result.body_markdown.contains("this website") || result.body_markdown.contains("website"),
        "anchor text should be present as plain text"
    );
}

#[test]
fn article_without_links_returns_full_body() {
    let result = extract_content(ARTICLE_WITHOUT_LINKS).unwrap();
    assert!(!result.body_markdown.is_empty());
    assert!(!result.body_markdown.contains("]("));
}

#[test]
fn h1_matching_page_title_is_not_duplicated_in_body() {
    let result = extract_content(ARTICLE_MATCHING_H1).unwrap();
    assert_eq!(result.title.as_deref(), Some("My Article"));
    // The leading h1 (matching the title) is stripped from the body so
    // callers can add their own heading via format_document without duplication.
    assert!(
        !result
            .body_markdown
            .trim_start()
            .starts_with("# My Article"),
        "leading h1 matching title should be stripped from body, got: {}",
        result.body_markdown
    );
}

#[test]
fn article_content_and_title_are_both_extracted() {
    // Trafilatura uses the prominent article heading (h1) as the metadata
    // title, not necessarily the HTML <title> tag. This verifies that both
    // title and body are returned for a normal article page.
    let result = extract_content(ARTICLE_DIFFERENT_H1).unwrap();
    assert!(result.title.is_some(), "title should be extracted");
    assert!(!result.body_markdown.is_empty(), "body should be non-empty");
    // Paragraph content should be present in the body regardless of how
    // trafilatura handles the heading.
    assert!(
        result.body_markdown.contains("Content under")
            || result.body_markdown.contains("heading that differs")
            || result.body_markdown.contains("enough text"),
        "body should contain article paragraph content, got: {}",
        result.body_markdown
    );
}

// ── kept: public API scenarios already meeting the standard ────────────

#[test]
fn extract_simple_html() {
    let html = r#"<html><head><title>Test Article</title></head>
        <body><article><h1>Test Article</h1>
        <p>This is a test article with enough content to pass the minimum length threshold for extraction.</p>
        </article></body></html>"#;
    let result = extract_content(html).unwrap();
    assert_eq!(result.title.as_deref(), Some("Test Article"));
    assert!(result.body_markdown.contains("test article"));
}

#[test]
fn extract_empty_returns_error() {
    let html = "<html><body></body></html>";
    let result = extract_content(html);
    assert!(result.is_err());
}
