// bo collect — the collect pipeline.
//
// Orchestrates the full flow for `bo collect <url>`: fetch HTML from the
// network, extract readable content, write the leaf file, and append to
// the index.
//
// Two entry points:
//
//   collect_url(url, output_dir)        — full pipeline including network fetch
//   collect_html(url, html, output_dir) — same, but accepts pre-fetched HTML
//
// `collect_html` is the testable core; `collect_url` is a thin wrapper that
// fetches first.
//
// Dependency direction: collect → adapters, fetch, quality, extract, leaf, slug, index.

use chrono::Utc;
use std::fmt;
use std::path::Path;

use crate::adapters::youtube::{self, YoutubeError, YoutubeUrlMatch};
use crate::domain::{index, leaf, slug};
use crate::engine::llm::models::DEFAULT_MODEL;
use crate::engine::quality::RejectReason;
use crate::engine::{extract, fetch, quality, summary};

// ── types ────────────────────────────────────────────────────────────────────

/// A document produced by the collect pipeline.
#[derive(Debug)]
pub struct Document {
    /// Normalised URL that was collected and recorded in the index.
    pub url: String,
    /// Filename (including `.md` extension) written inside `output_dir`.
    pub filename: String,
}

/// Unified error type for the collect pipeline.
#[derive(Debug)]
pub enum CollectError {
    /// The URL has already been collected; contains the existing filename.
    DuplicateUrl {
        existing_file: String,
    },
    Fetch(fetch::FetchError),
    Extract(extract::ExtractError),
    Youtube(YoutubeError),
    Summary(summary::SummaryError),
    Rejected {
        url: String,
        reason: RejectReason,
    },
    Io(std::io::Error),
}

impl fmt::Display for CollectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CollectError::DuplicateUrl { existing_file } => {
                write!(f, "already collected → {}", existing_file)
            }
            CollectError::Fetch(e) => write!(f, "{}", e),
            CollectError::Extract(e) => write!(f, "{}", e),
            CollectError::Youtube(e) => write!(f, "{}", e),
            CollectError::Summary(e) => write!(f, "{}", e),
            CollectError::Rejected { url, reason } => {
                write!(f, "{} was not collected: {}", url, reason)
            }
            CollectError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl From<fetch::FetchError> for CollectError {
    fn from(e: fetch::FetchError) -> Self {
        CollectError::Fetch(e)
    }
}

impl From<extract::ExtractError> for CollectError {
    fn from(e: extract::ExtractError) -> Self {
        CollectError::Extract(e)
    }
}

impl From<YoutubeError> for CollectError {
    fn from(e: YoutubeError) -> Self {
        CollectError::Youtube(e)
    }
}

impl From<summary::SummaryError> for CollectError {
    fn from(e: summary::SummaryError) -> Self {
        CollectError::Summary(e)
    }
}

impl From<std::io::Error> for CollectError {
    fn from(e: std::io::Error) -> Self {
        CollectError::Io(e)
    }
}

// ── pipeline ─────────────────────────────────────────────────────────────────

/// Full pipeline: validate URL, fetch HTML, then run the extract-write-ledger pipeline.
///
/// The `url` passed to the underlying `collect_html` call is the normalised form
/// returned by `fetch_url`, preserving the canonicalisation that was previously
/// done in `main.rs`.
pub fn collect_url(url: &str, output_dir: &Path) -> Result<Document, CollectError> {
    collect_url_with_model(url, output_dir, DEFAULT_MODEL)
}

pub fn collect_url_with_model(
    url: &str,
    output_dir: &Path,
    model: &str,
) -> Result<Document, CollectError> {
    match youtube::classify_url(url) {
        YoutubeUrlMatch::Supported(supported) => {
            ensure_not_duplicate(supported.normalized_url(), output_dir)?;
            let transcript = youtube::collect_transcript(url)?;
            return write_new_document_with_model(
                &transcript.url,
                Some(&transcript.title),
                &transcript.body_markdown,
                output_dir,
                model,
            );
        }
        YoutubeUrlMatch::Unsupported { url, reason } => {
            return Err(YoutubeError::UnsupportedUrl { url, reason }.into());
        }
        YoutubeUrlMatch::NotYoutube => {}
    }

    let fetched = match fetch::fetch_url(url) {
        Ok(fetched) => fetched,
        Err(fetch::FetchError::HttpStatus(status, message)) => {
            if let Some(reason) = quality::classify_http_status(status) {
                return Err(CollectError::Rejected {
                    url: url.to_string(),
                    reason,
                });
            }
            return Err(fetch::FetchError::HttpStatus(status, message).into());
        }
        Err(e) => return Err(e.into()),
    };
    collect_html_with_model(&fetched.url, &fetched.html, output_dir, model)
}

/// Extract-write-ledger pipeline without network access. Accepts pre-fetched HTML.
///
/// `url` is used for duplicate detection, slug generation, and the ledger entry.
/// It must be a valid, normalised URL string (e.g. as returned by `fetch_url`).
///
/// This is the testable core of the pipeline: integration tests call it directly
/// with fixture HTML to avoid network dependencies.
pub fn collect_html(url: &str, html: &str, output_dir: &Path) -> Result<Document, CollectError> {
    collect_html_with_model(url, html, output_dir, DEFAULT_MODEL)
}

pub fn collect_html_with_model(
    url: &str,
    html: &str,
    output_dir: &Path,
    model: &str,
) -> Result<Document, CollectError> {
    // Duplicate check — reads index only (fast path).
    // If index.jsonl is absent, the URL is treated as new.
    ensure_not_duplicate(url, output_dir)?;

    // Reject obvious non-document HTML before extraction.
    if let Some(reason) = quality::classify_html(html) {
        return Err(CollectError::Rejected {
            url: url.to_string(),
            reason,
        });
    }

    // Extract
    let content = extract::extract_content(html)?;

    // Reject extracted boilerplate/shell content before writing artifacts.
    if let Some(reason) =
        quality::classify_extracted(content.title.as_deref(), &content.body_markdown)
    {
        return Err(CollectError::Rejected {
            url: url.to_string(),
            reason,
        });
    }

    write_new_document_with_model(
        url,
        content.title.as_deref(),
        &content.body_markdown,
        output_dir,
        model,
    )
}

fn ensure_not_duplicate(url: &str, output_dir: &Path) -> Result<(), CollectError> {
    let index_path = output_dir.join("index.jsonl");
    let entries = index::read_index(&index_path)?;
    if let Some(existing) = index::is_duplicate(&entries, url) {
        return Err(CollectError::DuplicateUrl {
            existing_file: existing.file.clone(),
        });
    }
    Ok(())
}

fn write_new_document_with_model(
    url: &str,
    title: Option<&str>,
    body_markdown: &str,
    output_dir: &Path,
    model: &str,
) -> Result<Document, CollectError> {
    write_new_document_with_summary_result(
        url,
        title,
        body_markdown,
        output_dir,
        summary::generate(body_markdown, title, model),
    )
}

fn write_new_document_with_summary_result(
    url: &str,
    title: Option<&str>,
    body_markdown: &str,
    output_dir: &Path,
    summary_text: Result<String, summary::SummaryError>,
) -> Result<Document, CollectError> {
    let summary_text = summary_text?;
    let index_path = output_dir.join("index.jsonl");
    let title_ref = title.unwrap_or("");
    let base_slug = slug::slugify(title_ref, url);
    let filename = slug::resolve_slug(&base_slug, url, output_dir);

    // `leaf::write` calls `create_dir_all` internally, ensuring `output_dir`
    // exists before `append_entry` below requires the directory.
    let now_str = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let leaf_path = output_dir.join(format!("{}.md", filename));

    leaf::write(
        &leaf_path,
        title,
        url,
        &now_str,
        body_markdown,
        Some(&summary_text),
    )?;

    let entry = index::IndexEntry {
        file: format!("{}.md", filename),
        title: title.unwrap_or_default().to_string(),
        url: url.to_string(),
    };
    index::append_entry(&index_path, &entry)?;

    Ok(Document {
        url: url.to_string(),
        filename: format!("{}.md", filename),
    })
}

#[cfg(test)]
#[path = "../tests/cli_collect_tests.rs"]
mod tests;
