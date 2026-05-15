use super::*;
use std::fs;
use tempfile::TempDir;

const ARTICLE_HTML: &str = r#"<html><head><title>Plain Article</title></head>
<body><article><h1>Plain Article</h1>
<p>This article contains enough useful body text to pass extraction and quality
filtering. It remains an ordinary HTML collection fixture after refactoring.</p>
</article></body></html>"#;

#[test]
fn unsupported_youtube_embed_rejected_without_writes() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let result = collect_url("https://www.youtube.com/embed/a1mhk7mAetk", dir.path());

    assert!(matches!(result, Err(CollectError::Youtube(_))));
    assert!(!dir.path().join(".bo/index.jsonl").exists());
    let markdown_files = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "md"))
        .count();
    assert_eq!(markdown_files, 0);
}

#[test]
fn ordinary_html_collection_still_writes_leaf_and_index() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let document = collect_html("https://example.com/article", ARTICLE_HTML, dir.path()).unwrap();

    assert!(dir.path().join(&document.filename).exists());
    let entries = index::read_index(&dir.path().join(".bo/index.jsonl")).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].url, "https://example.com/article");
}

#[test]
fn summary_failure_writes_no_leaf_or_index_entry() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    let result = write_new_document_with_summary_result(
        "https://example.com/article",
        Some("Article"),
        "Substantial article body that would otherwise be written.",
        dir.path(),
        Err(summary::SummaryError::Parse("boom".to_string())),
    );

    assert!(matches!(result, Err(CollectError::Summary(_))));
    assert_no_collection_artifacts(&dir);
}

#[test]
fn collect_url_rejects_duplicate_youtube_url_before_network_fetch() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://www.youtube.com/watch?v=a1mhk7mAetk";
    index::append_entry(
        &dir.path().join(".bo/index.jsonl"),
        &index::IndexEntry {
            file: "existing.md".to_string(),
            title: "Existing Video".to_string(),
            url: url.to_string(),
        },
    )
    .unwrap();

    let duplicate = collect_url(url, dir.path());

    assert!(matches!(duplicate, Err(CollectError::DuplicateUrl { .. })));
    assert!(!dir.path().join("existing.md").exists());
}

#[test]
fn collect_html_keeps_exact_match_duplicate_semantics_for_youtube_urls() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    collect_html(
        "https://www.youtube.com/watch?v=a1mhk7mAetk",
        ARTICLE_HTML,
        dir.path(),
    )
    .unwrap();
    collect_html("https://youtu.be/a1mhk7mAetk", ARTICLE_HTML, dir.path()).unwrap();

    let entries = index::read_index(&dir.path().join(".bo/index.jsonl")).unwrap();
    assert_eq!(entries.len(), 2);
}

// ── pipeline integration tests (moved from tests/integration.rs) ─────────

const SAMPLE_HTML: &str = r#"
<html><head><title>Test Article</title></head>
<body><article>
<h1>Test Article</h1>
<p>This is a test article with substantial content that exceeds the minimum threshold for content extraction quality checks.</p>
<h2>Section One</h2>
<p>More detailed content about the first section of this test article, providing enough text for a meaningful extraction.</p>
</article></body></html>
"#;

const COLLISION_HTML_1: &str = r#"
<html><head><title>Introduction</title></head>
<body><article>
<h1>Introduction</h1>
<p>This is the first introduction page with enough content to pass the extraction threshold for quality filtering.</p>
</article></body></html>
"#;

const COLLISION_HTML_2: &str = r#"
<html><head><title>Introduction</title></head>
<body><article>
<h1>Introduction</h1>
<p>This is the second introduction page, from a completely different source, also with enough content for extraction.</p>
</article></body></html>
"#;

const REDIRECT_STUB_HTML: &str = r#"<!doctype html>
<meta charset="utf-8">
<title>Redirect</title>
<script>
  const target = "https://blog.rust-lang.org/2015/05/11/traits/";
  window.location.replace(target);
</script>
<noscript>
  <meta http-equiv="refresh" content="0; url=https://blog.rust-lang.org/2015/05/11/traits/">
</noscript>
<p><a href="https://blog.rust-lang.org/2015/05/11/traits/">Click here</a> to be redirected.</p>
"#;

const X_JS_SHELL_HTML: &str = r#"
<html><body>
<div class="errorContainer">
<h1>JavaScript is not available.</h1>
<p>We've detected that JavaScript is disabled in this browser. Please enable JavaScript or switch to a supported browser to continue using x.com.</p>
<p>Something went wrong, but don't fret — let's give it another shot.</p>
</div>
<div id="react-root"></div>
</body></html>
"#;

const CLOUDFLARE_BLOCK_HTML: &str = r#"
<html><head><title>Just a moment...</title>
<script src="https://challenges.cloudflare.com/turnstile/v0/api.js"></script></head>
<body><div id="cf-challenge">Checking your browser before accessing this site.</div></body></html>
"#;

const OPENREVIEW_FOOTER_HTML: &str = r#"
<html><head><title>ChainRepair: Enabling Efficient Program Repair with Small...</title></head>
<body><main>
<h1>ChainRepair: Enabling Efficient Program Repair with Small...</h1>
<p>OpenReview is a long-term project to advance science through improved peer review with legal nonprofit status. We gratefully acknowledge the support of the OpenReview Sponsors. © 2026 OpenReview</p>
</main></body></html>
"#;

const MDBOOK_WITH_BAD_UI_TITLE_HTML: &str = r#"
<html><head><title>Keyboard shortcuts</title></head>
<body>
<section class="help"><h2>Keyboard shortcuts</h2><p>Press ? to show keyboard shortcuts.</p></section>
<nav><h1>The Rust Programming Language</h1></nav>
<main>
<h1 id="understanding-ownership">Understanding Ownership</h1>
<p>Ownership is Rust's most unique feature and has deep implications for the rest of the language. It enables Rust to make memory safety guarantees without needing a garbage collector, so it is important to understand how ownership works.</p>
<p>This chapter discusses ownership, borrowing, slices, and how Rust lays data out in memory. The examples provide substantive documentation content that should be accepted even if surrounding UI chrome confuses title extraction.</p>
</main>
</body></html>
"#;

fn assert_no_collection_artifacts(dir: &TempDir) {
    let md_files: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "md"))
        .collect();
    assert!(
        md_files.is_empty(),
        "rejected collection wrote markdown files"
    );

    let index_path = dir.path().join(".bo/index.jsonl");
    assert!(
        !index_path.exists() || std::fs::read_to_string(&index_path).unwrap().is_empty(),
        "rejected collection wrote index entries"
    );
}

fn assert_rejected_with(result: Result<Document, CollectError>, url: &str, reason: &str) {
    let err = result
        .expect_err("collection should be rejected")
        .to_string();
    assert!(
        err.contains(&format!("{url} was not collected: {reason}")),
        "unexpected rejection message: {err}"
    );
}

#[test]
fn full_pipeline_happy_path() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let page = collect_html("https://example.com/article", SAMPLE_HTML, dir.path()).unwrap();

    assert!(dir.path().join(&page.filename).exists());

    let content = std::fs::read_to_string(dir.path().join(&page.filename)).unwrap();
    assert!(content.contains("title: \"Test Article\""));
    assert!(content.contains("url: https://example.com/article"));
    assert!(content.contains("collected_at:"));
    assert!(content.contains("updated_at:"));
    assert!(!content.contains("fetched:"));
    assert!(content.contains("# Test Article"));
    assert!(content.contains("Section One"));
    // Summary field is present (fallback: first ~200 words)
    assert!(content.contains("summary:"));

    let entries = index::read_index(&dir.path().join(".bo/index.jsonl")).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].url, "https://example.com/article");
}

#[test]
fn duplicate_rejected() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    collect_html("https://example.com/article", SAMPLE_HTML, dir.path()).unwrap();

    let result = collect_html("https://example.com/article", SAMPLE_HTML, dir.path());
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("already collected"));

    let entries = index::read_index(&dir.path().join(".bo/index.jsonl")).unwrap();
    assert_eq!(entries.len(), 1);
}

#[test]
fn slug_collision_disambiguated() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    let page1 = collect_html("https://example.com/intro1", COLLISION_HTML_1, dir.path()).unwrap();
    let page2 = collect_html("https://example.com/intro2", COLLISION_HTML_2, dir.path()).unwrap();

    assert!(dir.path().join(&page1.filename).exists());
    assert!(dir.path().join(&page2.filename).exists());
    assert_ne!(page1.filename, page2.filename);
    assert!(page1.filename.starts_with("introduction"));
    assert!(page2.filename.starts_with("introduction"));
    assert!(
        page2.filename.contains('-') && page2.filename.len() > page1.filename.len(),
        "second file should have hash suffix: {} vs {}",
        page1.filename,
        page2.filename
    );

    let entries = index::read_index(&dir.path().join(".bo/index.jsonl")).unwrap();
    assert_eq!(entries.len(), 2);
}

#[test]
fn empty_extraction_no_artifacts() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let empty_html = "<html><body></body></html>";

    let result = collect_html("https://example.com/empty", empty_html, dir.path());
    assert!(result.is_err());

    assert_no_collection_artifacts(&dir);
}

#[test]
fn redirect_stub_rejected_without_artifacts() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://blog.rust-lang.org/2015/05/11/traits.html";

    let result = collect_html(url, REDIRECT_STUB_HTML, dir.path());

    assert_rejected_with(result, url, "redirect stub");
    assert_no_collection_artifacts(&dir);
}

#[test]
fn x_js_shell_rejected_without_artifacts() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://x.com/lifeof_jer/status/2048103471019434248";

    let result = collect_html(url, X_JS_SHELL_HTML, dir.path());

    assert_rejected_with(result, url, "JS-rendered content");
    assert_no_collection_artifacts(&dir);
}

#[test]
fn openreview_footer_only_rejected_without_artifacts() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://openreview.net/forum?id=OAudWSf7aH";

    let result = collect_html(url, OPENREVIEW_FOOTER_HTML, dir.path());

    assert_rejected_with(result, url, "boilerplate-only content");
    assert_no_collection_artifacts(&dir);
}

#[test]
fn cloudflare_block_rejected_without_artifacts() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://medium.com/@loci.ai/deploying-vllm-on-ecs-with-ec2-82d58b482125";

    let result = collect_html(url, CLOUDFLARE_BLOCK_HTML, dir.path());

    assert_rejected_with(result, url, "blocked by site");
    assert_no_collection_artifacts(&dir);
}

#[test]
fn mdbook_page_with_bad_ui_title_and_substantive_body_is_accepted() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    let result = collect_html(
        "https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html",
        MDBOOK_WITH_BAD_UI_TITLE_HTML,
        dir.path(),
    );

    assert!(result.is_ok(), "mdBook page should be accepted: {result:?}");
    let page = result.unwrap();
    assert!(
        page.filename.starts_with("understanding-ownership"),
        "expected slug from content title, got {}",
        page.filename
    );

    let content = std::fs::read_to_string(dir.path().join(&page.filename)).unwrap();
    assert!(content.contains("title: \"Understanding Ownership\""));
    assert!(!content.contains("title: \"Keyboard shortcuts\""));

    let entries = index::read_index(&dir.path().join(".bo/index.jsonl")).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].title, "Understanding Ownership");
}

#[test]
fn failed_url_can_be_resubmitted() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let empty_html = "<html><body></body></html>";

    let result = collect_html("https://example.com/flaky", empty_html, dir.path());
    assert!(result.is_err());

    let result = collect_html("https://example.com/flaky", SAMPLE_HTML, dir.path());
    assert!(result.is_ok());
}

#[test]
fn near_duplicate_urls_both_stored() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    collect_html("https://example.com/article", SAMPLE_HTML, dir.path()).unwrap();
    collect_html(
        "https://example.com/article?ref=twitter",
        SAMPLE_HTML,
        dir.path(),
    )
    .unwrap();

    let entries = index::read_index(&dir.path().join(".bo/index.jsonl")).unwrap();
    assert_eq!(entries.len(), 2);
}
