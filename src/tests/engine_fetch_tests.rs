use super::*;

#[test]
fn invalid_scheme_rejected() {
    let result = fetch_url("ftp://example.com/file.txt");
    assert!(matches!(result, Err(FetchError::InvalidUrl(_))));
}

#[test]
fn unparseable_url_rejected() {
    let result = fetch_url("not a url at all");
    assert!(matches!(result, Err(FetchError::InvalidUrl(_))));
}

#[test]
#[ignore] // requires network
fn fetch_known_good_url() {
    let result = fetch_url("https://example.com").unwrap();
    assert!(result.html.contains("Example Domain"));
}

#[test]
#[ignore]
fn fetch_404() {
    let result = fetch_url("https://httpbin.org/status/404");
    assert!(matches!(result, Err(FetchError::HttpStatus(404, _))));
}

#[test]
#[ignore]
fn fetch_500_retries() {
    // This will retry 3 times due to 5xx
    let result = fetch_url("https://httpbin.org/status/500");
    assert!(matches!(result, Err(FetchError::HttpStatus(500, _))));
}

#[test]
#[ignore]
fn fetch_pdf_not_html() {
    let result =
        fetch_url("https://www.w3.org/WAI/ER/tests/xhtml/testfiles/resources/pdf/dummy.pdf");
    assert!(matches!(result, Err(FetchError::NotHtml(_))));
}

#[test]
#[ignore]
fn fetch_nonexistent_domain() {
    let result = fetch_url("https://this-domain-definitely-does-not-exist-abc123.com");
    assert!(matches!(result, Err(FetchError::Network(_))));
}
