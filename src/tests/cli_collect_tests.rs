// Orchestrator-level tests for the collect pipeline: the full expand→dedup→
// compute→commit flow driven through `run_pipeline`, `collect_with_compute`,
// and the `collect_batch_parallel_with_compute` test seam. Stage-unit tests
// (input/compute/commit/journal) live in their own files.
//
// This is the sole collect test file over 500 lines by design: every test here
// drives the whole pipeline and shares the `collect_html_test` /
// `seed_for_collect` / `assert_*` helpers and HTML fixtures; splitting it would
// duplicate those fixtures across files.

use super::*;
use crate::cli::collect::compute::compute_leaf_from_html;
use crate::domain::manifest;
use crate::domain::{Slug, Timestamp, Title, Url};
use std::fs;
use std::path::Path;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tempfile::TempDir;

fn collect_html_test(
    url: &str,
    html: &str,
    output_dir: &Path,
) -> Result<CollectResult, CollectError> {
    let url_s = url.to_string();
    let html_s = html.to_string();
    let outcomes = run_pipeline(
        vec![url_s.clone()],
        output_dir,
        &mut Vec::new(),
        move |_| compute_leaf_from_html(&url_s, &html_s, |_, _| "test summary".to_string()),
    )?;
    shape_single(outcomes)
}

#[test]
fn batch_collect_deduplicates_repeated_input_urls() {
    let dir = TempDir::new().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    let url = "https://example.com/article".to_string();

    let result = collect_batch_parallel_with_compute(
        vec![url.clone(), url.clone()],
        dir.path(),
        &mut Vec::new(),
        {
            let calls = Arc::clone(&calls);
            move |url| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(ComputedLeaf {
                    url: url.to_string(),
                    title: Some("Article".to_string()),
                    body_markdown: "body".to_string(),
                    summary_text: "summary".to_string(),
                    note_warning: None,
                })
            }
        },
    )
    .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.summary.collected, 1);
    assert_eq!(result.summary.skipped, 1);
    assert_eq!(result.summary.failed, 0);
    // Two-phase batch: phase-1 skips precede phase-3 successes, so the
    // duplicate (listed second) is items[0].
    assert_eq!(result.items[0].status, CollectItemStatus::Skipped);
    assert_eq!(result.items[0].code.as_deref(), Some("duplicate_input"));
}

#[test]
fn batch_collect_skips_existing_manifest_duplicates_without_fetching() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://example.com/already";

    // Write the URL into the manifest, the dedup source.
    let manifest_path = dir.path().join(".bo/manifest.json");
    crate::engine::manifest::write(
        &manifest_path,
        &manifest::Manifest {
            tree: manifest::TreeMeta {
                name: "test".to_string(),
                created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves: vec![crate::domain::Leaf {
                slug: Slug::parse("already").unwrap(),
                file: "already.md".to_string(),
                title: Some(Title::parse("Already").unwrap()),
                url: Url::parse(url).unwrap(),
                collected_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                summary: None,
            }],
            branches: Vec::new(),
        },
    )
    .unwrap();
    let result = collect_batch_parallel_with_compute(
        vec![url.to_string(), url.to_string()],
        dir.path(),
        &mut Vec::new(),
        move |_url| panic!("duplicate URL should not be fetched"),
    )
    .unwrap();

    assert_eq!(result.summary.collected, 0);
    assert_eq!(result.summary.skipped, 2);
    assert_eq!(result.summary.failed, 0);
    assert_eq!(result.items[0].code.as_deref(), Some("duplicate_url"));
    assert_eq!(result.items[0].existing_file.as_deref(), Some("already.md"));
    assert_eq!(result.items[1].code.as_deref(), Some("duplicate_input"));
}

#[test]
fn batch_collect_skips_same_batch_url_collapse() {
    let dir = TempDir::new().unwrap();
    let calls = Arc::new(AtomicUsize::new(0));
    // Two distinct input URLs that compute resolves to the same fetched URL
    // (e.g. both redirect to one canonical page). Phase-1 dedup keys on the
    // input URL, so both reach compute; phase-3 must catch the collision
    // against the in-memory manifest and skip the second without a second commit.
    let result = collect_batch_parallel_with_compute(
        vec![
            "https://example.com/a".to_string(),
            "https://example.com/b".to_string(),
        ],
        dir.path(),
        &mut Vec::new(),
        {
            let calls = Arc::clone(&calls);
            move |_url| {
                calls.fetch_add(1, Ordering::SeqCst);
                Ok(ComputedLeaf {
                    url: "https://example.com/canonical".to_string(),
                    title: Some("Canonical".to_string()),
                    body_markdown: "body".to_string(),
                    summary_text: "summary".to_string(),
                    note_warning: None,
                })
            }
        },
    )
    .unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(result.summary.collected, 1);
    assert_eq!(result.summary.skipped, 1);
    assert_eq!(result.summary.failed, 0);
    assert_eq!(result.items[1].status, CollectItemStatus::Skipped);
    assert_eq!(result.items[1].code.as_deref(), Some("duplicate_url"));
    assert_eq!(
        result.items[1].url.as_deref(),
        Some("https://example.com/canonical")
    );
    // existing_file must name the already-written leaf, not the
    // freshly-resolved (and never-written) filename for the duplicate.
    assert_eq!(
        result.items[1].existing_file.as_deref(),
        Some("leaf/canonical.md")
    );

    // Exactly one leaf committed despite two computes.
    let manifest = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(manifest.leaves.len(), 1);
    assert_eq!(
        manifest.leaves[0].url.as_str(),
        "https://example.com/canonical"
    );
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

    assert!(dir.path().join(&document.file).exists());
    let m = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 1);
    assert_eq!(m.leaves[0].url.as_str(), "https://example.com/article");
    assert!(!dir.path().join(".bo/index.jsonl").exists());
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

    let m = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
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

fn assert_rejected_with(result: Result<CollectResult, CollectError>, url: &str, reason: &str) {
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

    assert!(dir.path().join(&page.file).exists());

    let content = std::fs::read_to_string(dir.path().join(&page.file)).unwrap();
    // Assert via the parsed mapping so the test does not couple to serde's
    // conditional quoting of plain scalars (the behaviour #137 unified on).
    let (mapping, body) = crate::domain::frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some("Test Article")
    );
    assert_eq!(
        mapping.get("url").and_then(|v| v.as_str()),
        Some("https://example.com/article")
    );
    assert!(mapping.get("collected_at").is_some());
    assert!(mapping.get("fetched").is_none());
    assert!(body.contains("# Test Article"));
    assert!(body.contains("Section One"));
    // Summary and updated_at are NOT in leaf frontmatter — the manifest is the single source of truth.
    assert!(mapping.get("summary").is_none());
    assert!(mapping.get("updated_at").is_none());

    let m = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 1);
    assert_eq!(m.leaves[0].url.as_str(), "https://example.com/article");
    assert!(m.leaves[0].summary.is_some());
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

    let m = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
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

    assert!(dir.path().join(&page1.file).exists());
    assert!(dir.path().join(&page2.file).exists());
    assert_ne!(page1.file, page2.file);
    assert!(page1.file.starts_with("leaf/introduction"));
    assert!(page2.file.starts_with("leaf/introduction"));
    assert!(
        page2.file.contains('-') && page2.file.len() > page1.file.len(),
        "second file should have hash suffix: {} vs {}",
        page1.file,
        page2.file
    );

    let m = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
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
        page.file.starts_with("leaf/understanding-ownership"),
        "expected slug from content title, got {}",
        page.file
    );

    let content = std::fs::read_to_string(dir.path().join(&page.file)).unwrap();
    let (mapping, _) = crate::domain::frontmatter::parse(&content).unwrap();
    assert_eq!(
        mapping.get("title").and_then(|v| v.as_str()),
        Some("Understanding Ownership")
    );

    let m = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 1);
    assert_eq!(
        m.leaves[0].title.as_ref().unwrap().as_str(),
        "Understanding Ownership"
    );
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

    let m = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 2);
}

// ── manifest dual-write (T4.2) ────────────────────────────────────────────────────
//
// These tests bypass the real fetch+summarize pipeline by injecting a
// `ComputedLeaf` with synthetic data through `collect_batch_parallel_with_compute`
// (the test seam that accepts an injected compute closure).

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
    crate::engine::manifest::write(&manifest_path, &m).unwrap();
}

#[test]
fn collect_appends_leaf_record_to_manifest_with_full_metadata() {
    let dir = TempDir::new().unwrap();
    seed_for_collect(&dir, "manifest-collect-tree");

    let result = collect_batch_parallel_with_compute(
        vec!["https://example.com/article".to_string()],
        dir.path(),
        &mut Vec::new(),
        move |_| {
            Ok(ComputedLeaf {
                url: "https://example.com/article".to_string(),
                title: Some("Test Article".to_string()),
                body_markdown: "Substantial body about a topic.".to_string(),
                summary_text: "A summary of the article.".to_string(),
                note_warning: None,
            })
        },
    )
    .unwrap();

    let doc_file = result.items[0].file.clone().unwrap();
    let manifest_path = dir.path().join(".bo/manifest.json");
    let m = crate::engine::manifest::read(&manifest_path).unwrap();
    assert_eq!(m.leaves.len(), 1);
    let rec = &m.leaves[0];
    assert_eq!(
        rec.slug.as_str(),
        doc_file
            .strip_prefix("leaf/")
            .unwrap()
            .strip_suffix(".md")
            .unwrap()
    );
    assert_eq!(rec.file, doc_file);
    assert_eq!(rec.title.as_ref().unwrap().as_str(), "Test Article");
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
        let title = format!("Page {n}");
        let body = format!("Body for page {n}.");
        let summary = format!("Summary {n}");
        let result = collect_batch_parallel_with_compute(
            vec![url],
            dir.path(),
            &mut Vec::new(),
            move |_| {
                Ok(ComputedLeaf {
                    url: format!("https://example.com/page{n}"),
                    title: Some(title.clone()),
                    body_markdown: body.clone(),
                    summary_text: summary.clone(),
                    note_warning: None,
                })
            },
        )
        .unwrap();
        assert_eq!(result.summary.collected, 1);
    }

    let m = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();

    assert_eq!(m.leaves.len(), 3);
    assert!(!dir.path().join(".bo/index.jsonl").exists());
    for (n, rec) in m.leaves.iter().enumerate() {
        let n = n + 1;
        assert_eq!(rec.file, format!("leaf/page-{n}.md"));
        assert_eq!(rec.url.as_str(), &format!("https://example.com/page{n}"));
        assert_eq!(rec.title.as_ref().unwrap().as_str(), format!("Page {n}"));
    }
}

#[test]
fn collect_omits_summary_field_when_empty_string() {
    let dir = TempDir::new().unwrap();
    seed_for_collect(&dir, "empty-summary-tree");

    collect_batch_parallel_with_compute(
        vec!["https://example.com/article".to_string()],
        dir.path(),
        &mut Vec::new(),
        move |_| {
            Ok(ComputedLeaf {
                url: "https://example.com/article".to_string(),
                title: Some("Article".to_string()),
                body_markdown: "Body.".to_string(),
                summary_text: String::new(),
                note_warning: None,
            })
        },
    )
    .unwrap();

    let m = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert!(m.leaves[0].summary.is_none());
}

#[test]
fn fresh_collect_after_3b_does_not_write_index_secondary() {
    let dir = TempDir::new().unwrap();
    seed_for_collect(&dir, "secondary-still-written-tree");

    collect_batch_parallel_with_compute(
        vec!["https://example.com/page".to_string()],
        dir.path(),
        &mut Vec::new(),
        move |_| {
            Ok(ComputedLeaf {
                url: "https://example.com/page".to_string(),
                title: Some("Page".to_string()),
                body_markdown: "Body.".to_string(),
                summary_text: "Summary".to_string(),
                note_warning: None,
            })
        },
    )
    .unwrap();

    let m = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    assert_eq!(m.leaves.len(), 1);
    assert_eq!(m.leaves[0].url.as_str(), "https://example.com/page");
    assert!(!dir.path().join(".bo/index.jsonl").exists());
}

#[test]
fn recovery_notice_lands_in_warnings_not_stderr() {
    use crate::engine::pending;

    let dir = TempDir::new().unwrap();
    seed_for_collect(&dir, "recovery-warnings-tree");

    // Stage a stale pending op (dead pid, matching manifest hash → rollback path).
    let staged = b"rolled back";
    fs::write(dir.path().join("stale-leaf.md.tmp"), staged).unwrap();
    let op = pending::PendingOperation {
        op: pending::OpKind::Collect {
            url: "https://example.com/interrupted".to_string(),
        },
        started_at: "2020-01-01T00:00:00Z".to_string(),
        pid: 99999,
        pre_manifest_hash: pending::manifest_hash(dir.path()).unwrap(),
        writes: vec![pending::PendingWrite {
            path: "stale-leaf.md".to_string(),
            content_hash: pending::content_hash(staged),
        }],
        deletes: vec![],
    };
    pending::write(&dir.path().join(".bo/pending.json"), &op).unwrap();

    let mut warnings = Vec::new();
    collect_batch_parallel_with_compute(
        vec!["https://example.com/next".to_string()],
        dir.path(),
        &mut warnings,
        move |_| {
            Ok(ComputedLeaf {
                url: "https://example.com/next".to_string(),
                title: Some("Next".to_string()),
                body_markdown: "Body.".to_string(),
                summary_text: "Summary".to_string(),
                note_warning: None,
            })
        },
    )
    .unwrap();

    assert!(
        warnings.iter().any(|l| l.contains("recovered")),
        "recovery notice should land in warnings: {:?}",
        warnings
    );
}

// ── local markdown notes (#159) ─────────────────────────────────────────────

#[test]
fn collect_note_writes_leaf_with_no_summary_and_strips_frontmatter() {
    let dir = TempDir::new().unwrap();
    let md = dir.path().join("note.md");
    fs::write(&md, "---\nauthor: me\n---\n# A Note\n\nhello world").unwrap();

    let result = collect_batch_parallel_with_compute(
        vec![md.display().to_string()],
        dir.path(),
        &mut Vec::new(),
        move |_url| panic!("notes must not fetch"),
    )
    .unwrap();
    assert_eq!(result.summary.collected, 1);

    let item = &result.items[0];
    assert_eq!(item.status, CollectItemStatus::Collected);
    assert!(item.url.as_deref().unwrap().starts_with("bo://note/"));

    let written = fs::read_to_string(dir.path().join(item.file.as_deref().unwrap())).unwrap();
    assert!(written.contains("# A Note"));
    assert!(written.contains("hello world"));
    assert!(written.contains("url: bo://note/"));
    assert!(
        !written.contains("author"),
        "user frontmatter must be stripped"
    );

    // Notes skip the LLM summary: the manifest leaf carries no summary.
    let manifest = crate::engine::manifest::read(&dir.path().join(".bo/manifest.json")).unwrap();
    let leaf = manifest
        .leaves
        .iter()
        .find(|l| l.url.as_str().starts_with("bo://note/"))
        .expect("note leaf in manifest");
    assert!(leaf.summary.is_none());
}

// ── mixed-case .txt URL-list routing (#191 regression) ──────────────────────

#[test]
fn mixed_case_txt_list_returns_batch_with_every_url() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    // Nested mixed-case extension: is_url_list_file accepts .TXT
    // case-insensitively and reads it as a URL list. The output-policy check
    // must agree so all URLs are reported, not just the first.
    let list = dir.path().join("data/urls.TXT");
    fs::create_dir_all(list.parent().unwrap()).unwrap();
    fs::write(&list, "https://example.com/a\nhttps://example.com/b\n").unwrap();

    let result = collect_with_compute(
        vec![list.display().to_string()],
        dir.path(),
        "test-model",
        &mut Vec::new(),
        move |url| {
            Ok(ComputedLeaf {
                url: url.to_string(),
                title: Some("T".to_string()),
                body_markdown: "body".to_string(),
                summary_text: "s".to_string(),
                note_warning: None,
            })
        },
    )
    .unwrap();

    let batch = match result {
        CollectOutput::Batch(b) => b,
        other => panic!("mixed-case .txt list must be Batch, got {other:?}"),
    };
    assert_eq!(
        batch.summary.collected, 2,
        "every URL in the list must be collected"
    );
    assert_eq!(batch.items.len(), 2);
}

#[test]
fn empty_mixed_case_txt_list_is_batch_failure_not_panic() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    // An empty nested .TXT list expands to an empty_url_list failure item.
    // With correct routing this is a Batch failure; with Single routing it
    // would hit shape_single's Outcome::Item arm and panic (unreachable!).
    let list = dir.path().join("sub/empty.TXT");
    fs::create_dir_all(list.parent().unwrap()).unwrap();
    fs::write(&list, "\n  \n").unwrap();

    let result = collect_with_compute(
        vec![list.display().to_string()],
        dir.path(),
        "test-model",
        &mut Vec::new(),
        move |_| panic!("empty list must not be computed"),
    )
    .unwrap();

    let batch = match result {
        CollectOutput::Batch(b) => b,
        other => panic!("empty .txt list must be Batch, got {other:?}"),
    };
    assert_eq!(batch.summary.total, 1);
    assert_eq!(batch.summary.failed, 1);
    assert_eq!(batch.items[0].code.as_deref(), Some("empty_url_list"));
}
