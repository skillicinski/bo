// HTTP fetch with retry-backoff

use std::fmt;
use std::thread;
use std::time::Duration;
use tracing::warn;
use url::Url;

/// The result of a successful HTTP fetch.
/// `url` is the normalised (canonicalised) form of the requested URL.
pub struct FetchResult {
    pub url: String,
    pub html: String,
}

#[derive(Debug)]
pub enum FetchError {
    /// The URL could not be parsed or uses an unsupported scheme.
    InvalidUrl(String),
    Network(String),
    HttpStatus(u16, String),
    NotHtml(String),
    Timeout,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::InvalidUrl(msg) => write!(f, "invalid URL: {}", msg),
            FetchError::Network(msg) => write!(f, "network error: {}", msg),
            FetchError::HttpStatus(code, msg) => write!(f, "HTTP {}: {}", code, msg),
            FetchError::NotHtml(ct) => write!(f, "not HTML (Content-Type: {})", ct),
            FetchError::Timeout => write!(f, "request timed out"),
        }
    }
}

const MAX_RETRIES: u32 = 3;
const BACKOFF_BASE: u64 = 1; // seconds

fn is_retryable(err: &FetchError) -> bool {
    match err {
        FetchError::InvalidUrl(_) => false,
        FetchError::Timeout => true,
        FetchError::Network(_) => true,
        FetchError::HttpStatus(code, _) => *code >= 500,
        FetchError::NotHtml(_) => false,
    }
}

pub fn fetch_url(url: &str) -> Result<FetchResult, FetchError> {
    // Validate and normalise before any network attempt.
    let parsed = Url::parse(url)
        .map_err(|e| FetchError::InvalidUrl(format!("cannot parse '{}': {}", url, e)))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(FetchError::InvalidUrl(format!(
            "scheme '{}' is not http or https",
            parsed.scheme()
        )));
    }
    let canonical_url = parsed.as_str().to_string();

    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .user_agent("bo/0.1")
        .build()
        .map_err(|e| FetchError::Network(e.to_string()))?;

    let mut last_err = FetchError::Network("no attempts made".to_string());

    for attempt in 0..MAX_RETRIES {
        match try_fetch(&client, &canonical_url) {
            Ok(html) => {
                return Ok(FetchResult {
                    url: canonical_url,
                    html,
                })
            }
            Err(e) => {
                if !is_retryable(&e) {
                    return Err(e);
                }
                last_err = e;
                let next = attempt + 1;
                if next < MAX_RETRIES {
                    let delay = Duration::from_secs(BACKOFF_BASE * (1 << attempt));
                    warn!(
                        "attempt {}/{} failed ({}), retrying in {}s",
                        next,
                        MAX_RETRIES,
                        last_err,
                        delay.as_secs()
                    );
                    thread::sleep(delay);
                }
            }
        }
    }

    Err(last_err)
}

fn try_fetch(client: &reqwest::blocking::Client, url: &str) -> Result<String, FetchError> {
    let response = client.get(url).send().map_err(|e| {
        if e.is_timeout() {
            FetchError::Timeout
        } else {
            FetchError::Network(e.to_string())
        }
    })?;

    let status = response.status().as_u16();

    // Check HTTP status
    if status >= 400 {
        let reason = response.status().canonical_reason().unwrap_or("Unknown");
        return Err(FetchError::HttpStatus(status, reason.to_string()));
    }

    // Check Content-Type
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if !content_type.contains("text/html") && !content_type.contains("application/xhtml+xml") {
        return Err(FetchError::NotHtml(content_type.to_string()));
    }

    let html = response
        .text()
        .map_err(|e| FetchError::Network(e.to_string()))?;

    Ok(html)
}

#[cfg(test)]
mod tests {
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
}
