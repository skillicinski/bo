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
use crate::{extract, fetch, index, leaf, quality, slug, RejectReason};

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
    match youtube::classify_url(url) {
        YoutubeUrlMatch::Supported(_) => {
            let transcript = youtube::collect_transcript(url)?;
            return write_document(
                &transcript.url,
                Some(&transcript.title),
                &transcript.body_markdown,
                output_dir,
            );
        }
        YoutubeUrlMatch::Unsupported { url, reason } => {
            return Err(YoutubeError::UnsupportedUrl { url, reason }.into());
        }
        YoutubeUrlMatch::Invalid { message } => {
            return Err(YoutubeError::InvalidUrl(message).into())
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
    collect_html(&fetched.url, &fetched.html, output_dir)
}

/// Extract-write-ledger pipeline without network access. Accepts pre-fetched HTML.
///
/// `url` is used for duplicate detection, slug generation, and the ledger entry.
/// It must be a valid, normalised URL string (e.g. as returned by `fetch_url`).
///
/// This is the testable core of the pipeline: integration tests call it directly
/// with fixture HTML to avoid network dependencies.
pub fn collect_html(url: &str, html: &str, output_dir: &Path) -> Result<Document, CollectError> {
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

    write_new_document(
        url,
        content.title.as_deref(),
        &content.body_markdown,
        output_dir,
    )
}

fn write_document(
    url: &str,
    title: Option<&str>,
    body_markdown: &str,
    output_dir: &Path,
) -> Result<Document, CollectError> {
    ensure_not_duplicate(url, output_dir)?;
    write_new_document(url, title, body_markdown, output_dir)
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

fn write_new_document(
    url: &str,
    title: Option<&str>,
    body_markdown: &str,
    output_dir: &Path,
) -> Result<Document, CollectError> {
    let index_path = output_dir.join("index.jsonl");
    let title_ref = title.unwrap_or("");
    let base_slug = slug::slugify(title_ref, url);
    let filename = slug::resolve_slug(&base_slug, url, output_dir);

    // `leaf::write` calls `create_dir_all` internally, ensuring `output_dir`
    // exists before `append_entry` below requires the directory.
    let now_str = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let leaf_path = output_dir.join(format!("{}.md", filename));
    leaf::write(&leaf_path, title, url, &now_str, body_markdown)?;

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
mod tests {
    use super::*;
    use tempfile::TempDir;

    const ARTICLE_HTML: &str = r#"<html><head><title>Plain Article</title></head>
<body><article><h1>Plain Article</h1>
<p>This article contains enough useful body text to pass extraction and quality
filtering. It remains an ordinary HTML collection fixture after refactoring.</p>
</article></body></html>"#;

    #[test]
    fn unsupported_youtube_embed_rejected_without_writes() {
        let dir = TempDir::new().unwrap();
        let result = collect_url("https://www.youtube.com/embed/a1mhk7mAetk", dir.path());

        assert!(matches!(result, Err(CollectError::Youtube(_))));
        assert!(!dir.path().join("index.jsonl").exists());
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
        let document =
            collect_html("https://example.com/article", ARTICLE_HTML, dir.path()).unwrap();

        assert!(dir.path().join(&document.filename).exists());
        let entries = index::read_index(&dir.path().join("index.jsonl")).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "https://example.com/article");
    }

    #[test]
    fn shared_write_path_detects_exact_duplicate_youtube_url() {
        let dir = TempDir::new().unwrap();
        let url = "https://www.youtube.com/watch?v=a1mhk7mAetk";

        write_document(url, Some("Video Title"), "Transcript body", dir.path()).unwrap();
        let duplicate = write_document(url, Some("Video Title"), "Transcript body", dir.path());

        assert!(matches!(duplicate, Err(CollectError::DuplicateUrl { .. })));
        let entries = index::read_index(&dir.path().join("index.jsonl")).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn shared_write_path_keeps_exact_match_duplicate_semantics() {
        let dir = TempDir::new().unwrap();

        write_document(
            "https://www.youtube.com/watch?v=a1mhk7mAetk",
            Some("Video Title"),
            "Transcript body",
            dir.path(),
        )
        .unwrap();
        write_document(
            "https://youtu.be/a1mhk7mAetk",
            Some("Video Title"),
            "Transcript body",
            dir.path(),
        )
        .unwrap();

        let entries = index::read_index(&dir.path().join("index.jsonl")).unwrap();
        assert_eq!(entries.len(), 2);
    }
}
