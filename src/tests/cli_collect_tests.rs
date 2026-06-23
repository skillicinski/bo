use super::*;
use crate::domain::manifest;
use crate::domain::{Slug, Timestamp};
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn collect_html_test(url: &str, html: &str, output_dir: &Path) -> Result<Document, CollectError> {
    collect_html_with_summarizer(url, html, output_dir, |_, _| Ok("test summary".to_string()))
}

#[test]
fn collect_input_expands_txt_url_list() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("urls.txt");
    fs::write(
        &path,
        " https://example.com/one \n\nhttps://example.com/two\n",
    )
    .unwrap();

    let expanded = expand_collect_inputs(&[path.display().to_string()]);

    assert_eq!(expanded.len(), 2);
    match &expanded[0] {
        ExpandedCollectInput::Url {
            input,
            url,
            from_file,
        } => {
            assert!(input.ends_with("urls.txt:1"), "input was {input}");
            assert_eq!(url, "https://example.com/one");
            assert!(*from_file);
        }
        other => panic!("unexpected expanded input: {other:?}"),
    }
    match &expanded[1] {
        ExpandedCollectInput::Url { input, url, .. } => {
            assert!(input.ends_with("urls.txt:3"), "input was {input}");
            assert_eq!(url, "https://example.com/two");
        }
        other => panic!("unexpected expanded input: {other:?}"),
    }
}

#[test]
fn collect_input_treats_missing_local_txt_as_url_list_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("missing.txt");

    let expanded = expand_collect_inputs(&[path.display().to_string()]);

    assert_eq!(expanded.len(), 1);
    match &expanded[0] {
        ExpandedCollectInput::Failure { item, from_file } => {
            assert!(*from_file);
            assert_eq!(item.status, CollectItemStatus::Failed);
            assert_eq!(item.code.as_deref(), Some("url_list_read_error"));
        }
        other => panic!("unexpected expanded input: {other:?}"),
    }
}

#[test]
fn batch_collect_deduplicates_repeated_input_urls() {
    let dir = TempDir::new().unwrap();
    let mut calls = 0;
    let url = "https://example.com/article".to_string();

    let result = collect_inputs_with_collector(
        vec![url.clone(), url.clone()],
        dir.path(),
        |collected_url| {
            calls += 1;
            Ok(Document {
                url: collected_url.to_string(),
                filename: format!("article-{calls}.md"),
            })
        },
    )
    .unwrap();

    let CollectOutput::Batch(result) = result else {
        panic!("expected batch result");
    };
    assert_eq!(calls, 1);
    assert_eq!(result.summary.collected, 1);
    assert_eq!(result.summary.skipped, 1);
    assert_eq!(result.summary.failed, 0);
    assert_eq!(result.items[1].status, CollectItemStatus::Skipped);
    assert_eq!(result.items[1].code.as_deref(), Some("duplicate_input"));
}

#[test]
fn batch_collect_skips_existing_manifest_duplicates_without_fetching() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://example.com/already";

    // Write the URL into the manifest, the dedup source.
    let manifest_path = dir.path().join(".bo/manifest.json");
    manifest::write(
        &manifest_path,
        &manifest::Manifest {
            tree: manifest::TreeMeta {
                name: "test".to_string(),
                created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves: vec![manifest::LeafRecord {
                slug: Slug::parse("already").unwrap(),
                file: "already.md".to_string(),
                title: ("Already").to_string(),
                url: (url).to_string(),
                collected_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                summary: None,
            }],
            branches: Vec::new(),
        },
    )
    .unwrap();
    let result =
        collect_inputs_with_collector(vec![url.to_string(), url.to_string()], dir.path(), |_url| {
            panic!("duplicate URL should not be fetched")
        })
        .unwrap();

    let CollectOutput::Batch(result) = result else {
        panic!("expected batch result");
    };
    assert_eq!(result.summary.collected, 0);
    assert_eq!(result.summary.skipped, 2);
    assert_eq!(result.summary.failed, 0);
    assert_eq!(result.items[0].code.as_deref(), Some("duplicate_url"));
    assert_eq!(result.items[0].existing_file.as_deref(), Some("already.md"));
    assert_eq!(result.items[1].code.as_deref(), Some("duplicate_input"));
}

const ARTICLE_HTML: &str = r#"<html><head><title>Plain Article</title></head>
<body><article><h1>Plain Article</h1>
<p>This article contains enough useful body text to pass extraction and quality
filtering. It remains an ordinary HTML collection fixture after refactoring.</p>
</article></body></html>"#;

#[test]
fn ordinary_html_collection_writes_leaf_and_manifest() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let document =
        collect_html_test("https://example.com/article", ARTICLE_HTML, dir.path()).unwrap();

    assert!(dir.path().join(&document.filename).exists());
    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 1);
    assert_eq!(m.leaves[0].url.as_str(), "https://example.com/article");
    assert!(!dir.path().join(".bo/index.jsonl").exists());
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
fn collect_html_keeps_exact_match_duplicate_semantics_for_youtube_urls() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    collect_html_test(
        "https://www.youtube.com/watch?v=a1mhk7mAetk",
        ARTICLE_HTML,
        dir.path(),
    )
    .unwrap();
    collect_html_test("https://youtu.be/a1mhk7mAetk", ARTICLE_HTML, dir.path()).unwrap();

    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 2);
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
    let page = collect_html_test("https://example.com/article", SAMPLE_HTML, dir.path()).unwrap();

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

    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 1);
    assert_eq!(m.leaves[0].url.as_str(), "https://example.com/article");
}

#[test]
fn duplicate_rejected() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    collect_html_test("https://example.com/article", SAMPLE_HTML, dir.path()).unwrap();

    let result = collect_html_test("https://example.com/article", SAMPLE_HTML, dir.path());
    assert!(result.is_err());
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("already collected"));

    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 1);
}

#[test]
fn slug_collision_disambiguated() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    let page1 =
        collect_html_test("https://example.com/intro1", COLLISION_HTML_1, dir.path()).unwrap();
    let page2 =
        collect_html_test("https://example.com/intro2", COLLISION_HTML_2, dir.path()).unwrap();

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

    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 2);
}

#[test]
fn empty_extraction_no_artifacts() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let empty_html = "<html><body></body></html>";

    let result = collect_html_test("https://example.com/empty", empty_html, dir.path());
    assert!(result.is_err());

    assert_no_collection_artifacts(&dir);
}

#[test]
fn redirect_stub_rejected_without_artifacts() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://blog.rust-lang.org/2015/05/11/traits.html";

    let result = collect_html_test(url, REDIRECT_STUB_HTML, dir.path());

    assert_rejected_with(result, url, "redirect stub");
    assert_no_collection_artifacts(&dir);
}

#[test]
fn x_js_shell_rejected_without_artifacts() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://x.com/lifeof_jer/status/2048103471019434248";

    let result = collect_html_test(url, X_JS_SHELL_HTML, dir.path());

    assert_rejected_with(result, url, "JS-rendered content");
    assert_no_collection_artifacts(&dir);
}

#[test]
fn openreview_footer_only_rejected_without_artifacts() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://openreview.net/forum?id=OAudWSf7aH";

    let result = collect_html_test(url, OPENREVIEW_FOOTER_HTML, dir.path());

    assert_rejected_with(result, url, "boilerplate-only content");
    assert_no_collection_artifacts(&dir);
}

#[test]
fn cloudflare_block_rejected_without_artifacts() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://medium.com/@loci.ai/deploying-vllm-on-ecs-with-ec2-82d58b482125";

    let result = collect_html_test(url, CLOUDFLARE_BLOCK_HTML, dir.path());

    assert_rejected_with(result, url, "blocked by site");
    assert_no_collection_artifacts(&dir);
}

#[test]
fn mdbook_page_with_bad_ui_title_and_substantive_body_is_accepted() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    let result = collect_html_test(
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

    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 1);
    assert_eq!(m.leaves[0].title.as_str(), "Understanding Ownership");
}

#[test]
fn failed_url_can_be_resubmitted() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let empty_html = "<html><body></body></html>";

    let result = collect_html_test("https://example.com/flaky", empty_html, dir.path());
    assert!(result.is_err());

    let result = collect_html_test("https://example.com/flaky", SAMPLE_HTML, dir.path());
    assert!(result.is_ok());
}

#[test]
fn near_duplicate_urls_both_stored() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    collect_html_test("https://example.com/article", SAMPLE_HTML, dir.path()).unwrap();
    collect_html_test(
        "https://example.com/article?ref=twitter",
        SAMPLE_HTML,
        dir.path(),
    )
    .unwrap();

    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 2);
}

// ── manifest dual-write (T4.2) ────────────────────────────────────────────────────
//
// These tests bypass `collect_html` (which calls `summary::generate` and so
// requires real OpenAI auth) by invoking the post-extraction pipeline
// `write_new_document_with_summary_result` with a synthetic `Ok` summary.

fn seed_for_collect(dir: &TempDir, name: &str) {
    use crate::domain::manifest::TreeMeta;
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let manifest_path = dir.path().join(".bo/manifest.json");
    let m = manifest::Manifest {
        tree: TreeMeta {
            name: name.to_string(),
            created_at: Timestamp::parse("2026-05-19T12:00:00Z").unwrap(),
            last_compiled_at: None,
        },
        leaves: Vec::new(),
        branches: Vec::new(),
    };
    manifest::write(&manifest_path, &m).unwrap();
}

#[test]
fn collect_appends_leaf_record_to_manifest_with_full_metadata() {
    let dir = TempDir::new().unwrap();
    seed_for_collect(&dir, "manifest-collect-tree");

    let doc = write_new_document_with_summary_result(
        "https://example.com/article",
        Some("Test Article"),
        "Substantial body about a topic.",
        dir.path(),
        Ok("A summary of the article.".to_string()),
    )
    .unwrap();

    let manifest_path = dir.path().join(".bo/manifest.json");
    let m = manifest::read(&manifest_path).unwrap();
    assert_eq!(m.leaves.len(), 1);
    let rec = &m.leaves[0];
    assert_eq!(rec.slug.as_str(), doc.filename.strip_suffix(".md").unwrap());
    assert_eq!(rec.file, doc.filename);
    assert_eq!(rec.title.as_str(), "Test Article");
    assert_eq!(rec.url.as_str(), "https://example.com/article");
    assert!(
        rec.collected_at.to_string().contains('T'),
        "collected_at iso8601: {}",
        rec.collected_at
    );
    assert_eq!(rec.summary.as_deref(), Some("A summary of the article."));
    // Tree metadata preserved across the dual-write.
    assert_eq!(m.tree.name, "manifest-collect-tree");
    assert!(m.tree.last_compiled_at.is_none());
}

#[test]
fn collect_writes_only_manifest_records() {
    let dir = TempDir::new().unwrap();
    seed_for_collect(&dir, "parity-tree");

    for n in 1..=3 {
        let url = format!("https://example.com/page{n}");
        write_new_document_with_summary_result(
            &url,
            Some(&format!("Page {n}")),
            &format!("Body for page {n}."),
            dir.path(),
            Ok(format!("Summary {n}")),
        )
        .unwrap();
    }

    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();

    assert_eq!(m.leaves.len(), 3);
    assert!(!dir.path().join(".bo/index.jsonl").exists());
    for (n, rec) in m.leaves.iter().enumerate() {
        let n = n + 1;
        assert_eq!(rec.file, format!("page-{n}.md"));
        assert_eq!(rec.url.as_str(), &format!("https://example.com/page{n}"));
        assert_eq!(rec.title.as_str(), format!("Page {n}"));
    }
}

#[test]
fn collect_omits_summary_field_when_empty_string() {
    let dir = TempDir::new().unwrap();
    seed_for_collect(&dir, "empty-summary-tree");

    write_new_document_with_summary_result(
        "https://example.com/article",
        Some("Article"),
        "Body.",
        dir.path(),
        Ok(String::new()),
    )
    .unwrap();

    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert!(m.leaves[0].summary.is_none());
}

#[test]
fn dedup_uses_manifest_not_index_jsonl() {
    let dir = TempDir::new().unwrap();
    seed_for_collect(&dir, "manifest-dedup-tree");

    // Pre-populate the manifest with a leaf, but NOT the index. The dedup
    // path used by ensure_not_duplicate must consult the manifest now.
    let manifest_path = dir.path().join(".bo/manifest.json");
    let mut m = manifest::read(&manifest_path).unwrap();
    m.leaves.push(crate::domain::manifest::LeafRecord {
        slug: Slug::parse("already-collected").unwrap(),
        file: "already-collected.md".to_string(),
        title: ("Already").to_string(),
        url: ("https://example.com/article").to_string(),
        collected_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        summary: None,
    });
    manifest::write(&manifest_path, &m).unwrap();

    // Verify duplicate_file (the dedup helper) finds it via manifest only.
    let existing = duplicate_file("https://example.com/article", dir.path()).unwrap();
    assert_eq!(existing.as_deref(), Some("already-collected.md"));

    // Sanity: a different URL is not flagged.
    let none = duplicate_file("https://example.com/other", dir.path()).unwrap();
    assert!(none.is_none());
}

#[test]
fn fresh_collect_after_3b_does_not_write_index_secondary() {
    let dir = TempDir::new().unwrap();
    seed_for_collect(&dir, "secondary-still-written-tree");

    write_new_document_with_summary_result(
        "https://example.com/page",
        Some("Page"),
        "Body.",
        dir.path(),
        Ok("Summary".to_string()),
    )
    .unwrap();

    let m = manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 1);
    assert_eq!(m.leaves[0].url.as_str(), "https://example.com/page");
    assert!(!dir.path().join(".bo/index.jsonl").exists());
}
