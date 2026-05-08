//! Collection quality classification.
//!
//! These checks reject strong signals that fetched/extracted content is not an
//! acceptable document. They are intentionally conservative: valid articles that
//! merely mention JavaScript, captchas, redirects, or Cloudflare should not be
//! rejected by keyword presence alone.

use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectReason {
    BlockedBySite,
    JsRenderedContent,
    RedirectStub,
    BoilerplateOnlyContent,
}

impl fmt::Display for RejectReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RejectReason::BlockedBySite => write!(f, "blocked by site"),
            RejectReason::JsRenderedContent => write!(f, "JS-rendered content"),
            RejectReason::RedirectStub => write!(f, "redirect stub"),
            RejectReason::BoilerplateOnlyContent => write!(f, "boilerplate-only content"),
        }
    }
}

pub fn classify_http_status(status: u16) -> Option<RejectReason> {
    match status {
        401 | 403 | 429 => Some(RejectReason::BlockedBySite),
        _ => None,
    }
}

pub fn classify_html(html: &str) -> Option<RejectReason> {
    let text = normalize(html);

    if is_block_challenge_html(&text) {
        return Some(RejectReason::BlockedBySite);
    }
    if is_redirect_stub_html(&text) {
        return Some(RejectReason::RedirectStub);
    }
    if is_js_required_shell_html(&text) {
        return Some(RejectReason::JsRenderedContent);
    }

    None
}

pub fn classify_extracted(title: Option<&str>, body_markdown: &str) -> Option<RejectReason> {
    let title = title.map(normalize).unwrap_or_default();
    let body = normalize(body_markdown);

    if is_redirect_stub_extracted(&title, &body) {
        return Some(RejectReason::RedirectStub);
    }
    if is_js_required_shell_extracted(&body) {
        return Some(RejectReason::JsRenderedContent);
    }
    if is_block_challenge_extracted(&body) {
        return Some(RejectReason::BlockedBySite);
    }
    if is_boilerplate_only_extracted(&body) {
        return Some(RejectReason::BoilerplateOnlyContent);
    }

    None
}

fn normalize(input: &str) -> String {
    input
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| haystack.contains(needle))
}

fn contains_all(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().all(|needle| haystack.contains(needle))
}

fn word_count(text: &str) -> usize {
    text.split_whitespace().count()
}

fn is_redirect_stub_html(text: &str) -> bool {
    let has_strong_redirect_mechanism = contains_any(
        text,
        &[
            "http-equiv=\"refresh\"",
            "http-equiv='refresh'",
            "http-equiv=refresh",
            "window.location.replace",
            "location.replace(",
        ],
    );
    let has_href_assignment = contains_any(text, &["window.location.href", "location.href="]);
    let has_redirect_title = contains_any(text, &["<title>redirect", ">redirect</title>"]);
    let has_redirect_body = contains_any(text, &["click here", "to be redirected"]);

    let has_redirect_mechanism = has_strong_redirect_mechanism
        || (has_href_assignment && (has_redirect_title || has_redirect_body));
    let has_redirect_shell = has_redirect_title || has_redirect_body;

    has_redirect_mechanism && has_redirect_shell
}

fn is_js_required_shell_html(text: &str) -> bool {
    text.contains("javascript is not available")
        || contains_all(text, &["please enable javascript", "supported browser"])
        || contains_all(text, &["javascript is disabled", "supported browser"])
        || contains_all(text, &["enable javascript", "continue using", "x.com"])
        || contains_all(
            text,
            &["enable javascript", "react-root", "something went wrong"],
        )
}

fn is_block_challenge_html(text: &str) -> bool {
    contains_any(
        text,
        &["cf-mitigated", "cf-challenge", "cf-browser-verification"],
    ) || contains_all(text, &["challenges.cloudflare.com", "cf-ray"])
        || contains_all(text, &["just a moment", "checking your browser"])
        || contains_all(text, &["verify you are human", "captcha"])
        || contains_all(text, &["access denied", "captcha"])
        || contains_all(text, &["access denied", "cloudflare"])
        || contains_all(text, &["unusual traffic", "captcha"])
}

fn is_redirect_stub_extracted(title: &str, body: &str) -> bool {
    (title == "redirect" || body.starts_with("# redirect") || body.starts_with("redirect"))
        && contains_any(body, &["click here", "to be redirected"])
}

fn is_js_required_shell_extracted(body: &str) -> bool {
    body.contains("javascript is not available")
        || contains_all(body, &["please enable javascript", "supported browser"])
        || contains_all(body, &["javascript is disabled", "supported browser"])
}

fn is_block_challenge_extracted(body: &str) -> bool {
    contains_all(body, &["just a moment", "checking your browser"])
        || contains_all(body, &["verify you are human", "captcha"])
        || contains_all(body, &["access denied", "captcha"])
        || contains_all(body, &["access denied", "cloudflare"])
}

fn is_boilerplate_only_extracted(body: &str) -> bool {
    let words = word_count(body);

    let openreview_footer = contains_all(
        body,
        &["openreview is a long-term project", "openreview sponsors"],
    );
    if openreview_footer && !contains_any(body, &["abstract", "introduction", "we present"]) {
        return true;
    }

    let legal_footer = contains_all(body, &["terms of service", "privacy policy"])
        && contains_any(body, &["cookie policy", "ads info", "all rights reserved"]);
    if legal_footer && words <= 120 {
        return true;
    }

    words <= 80
        && contains_any(
            body,
            &[
                "all rights reserved",
                "copyright",
                "terms of use",
                "privacy policy",
            ],
        )
        && !contains_any(body, &["abstract", "introduction", "chapter", "article"])
}

#[cfg(test)]
mod tests {
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
<p>We’ve detected that JavaScript is disabled in this browser. Please enable JavaScript or switch to a supported browser to continue using x.com.</p>
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
        let body = "# Understanding Ownership\n\nOwnership is Rust’s most unique feature and has deep implications for the rest of the language. It enables Rust to make memory safety guarantees without needing a garbage collector. This chapter explains ownership, borrowing, slices, and how Rust lays data out in memory with concrete examples and detailed discussion.";

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
}
