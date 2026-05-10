use super::*;

#[test]
fn blocked_statuses_are_rejected() {
    assert_eq!(classify_http_status(401), Some(RejectReason::BlockedBySite));
    assert_eq!(classify_http_status(403), Some(RejectReason::BlockedBySite));
    assert_eq!(classify_http_status(429), Some(RejectReason::BlockedBySite));
    assert_eq!(classify_http_status(404), None);
    assert_eq!(classify_http_status(500), None);
}

#[test]
fn redirect_stub_html_is_rejected() {
    let html = r#"<!doctype html>
<meta charset="utf-8">
<title>Redirect</title>
<script>window.location.replace("https://example.com/target/");</script>
<noscript><meta http-equiv="refresh" content="0; url=https://example.com/target/"></noscript>
<p><a href="https://example.com/target/">Click here</a> to be redirected.</p>"#;

    assert_eq!(classify_html(html), Some(RejectReason::RedirectStub));
}

#[test]
fn js_required_shell_html_is_rejected() {
    let html = r#"<html><body>
<div class="errorContainer">
<h1>JavaScript is not available.</h1>
<p>We've detected that JavaScript is disabled in this browser. Please enable JavaScript or switch to a supported browser to continue using x.com.</p>
</div>
</body></html>"#;

    assert_eq!(classify_html(html), Some(RejectReason::JsRenderedContent));
}

#[test]
fn block_challenge_html_is_rejected() {
    let html = r#"<html><head><title>Just a moment...</title>
<script src="https://challenges.cloudflare.com/turnstile/v0/api.js"></script></head>
<body><div id="cf-challenge">Checking your browser before accessing this site.</div></body></html>"#;

    assert_eq!(classify_html(html), Some(RejectReason::BlockedBySite));
}

#[test]
fn openreview_footer_only_extracted_content_is_rejected() {
    let body = "OpenReview is a long-term project to advance science through improved peer review with legal nonprofit status. We gratefully acknowledge the support of the OpenReview Sponsors. © 2026 OpenReview";

    assert_eq!(
        classify_extracted(
            Some("ChainRepair: Enabling Efficient Program Repair with Small..."),
            body
        ),
        Some(RejectReason::BoilerplateOnlyContent)
    );
}

#[test]
fn generic_or_wrong_title_with_substantive_body_is_accepted() {
    let body = "# Understanding Ownership\n\nOwnership is Rust's most unique feature and has deep implications for the rest of the language. It enables Rust to make memory safety guarantees without needing a garbage collector. This chapter explains ownership, borrowing, slices, and how Rust lays data out in memory with concrete examples and detailed discussion.";

    assert_eq!(classify_extracted(Some("Keyboard shortcuts"), body), None);
}

#[test]
fn articles_merely_mentioning_suspicious_terms_are_accepted() {
    let html = r#"<html><head><title>Browser Security Patterns</title></head>
<body><article><h1>Browser Security Patterns</h1>
<p>This article discusses JavaScript, captcha systems, redirects, and Cloudflare as examples of web platform tradeoffs. It is ordinary article prose, not an access challenge or application shell.</p>
<p>The important distinction is whether the document itself is available as static content.</p>
</article></body></html>"#;
    let body = "This article discusses JavaScript, captcha systems, redirects, and Cloudflare as examples of web platform tradeoffs. It is ordinary article prose, not an access challenge or application shell. The important distinction is whether the document itself is available as static content.";

    assert_eq!(classify_html(html), None);
    assert_eq!(
        classify_extracted(Some("Browser Security Patterns"), body),
        None
    );
}

#[test]
fn article_html_with_bundled_location_href_is_not_a_redirect_stub() {
    let html = r#"<html><head><title>How to write a good spec for AI agents</title></head>
<body><article><h1>How to write a good spec for AI agents</h1>
<p>This article explains how specifications improve AI coding agent workflows with concrete examples and practical guidance.</p></article>
<script>function focusFrame(t){ return t.contentWindow.location.href; }</script>
<script>window.CustomSubstackWidget = { substackUrl: "example.substack.com" }; // custom redirect</script>
</body></html>"#;

    assert_eq!(classify_html(html), None);
}

#[test]
fn extracted_redirect_stub_is_rejected() {
    let body = "Click here to be redirected. Redirect <meta http-equiv=\"refresh\" content=\"0; url=https://example.com/target/\"> Click here to be redirected.";

    assert_eq!(
        classify_extracted(Some("Redirect"), body),
        Some(RejectReason::RedirectStub)
    );
}
