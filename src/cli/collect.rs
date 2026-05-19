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
use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::Path;

use crate::adapters::youtube::{self, YoutubeError, YoutubeUrlMatch};
use crate::domain::{index, leaf, slug, tree};
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

pub fn error_code(error: &CollectError) -> &'static str {
    match error {
        CollectError::DuplicateUrl { .. } => "duplicate_url",
        CollectError::Rejected { .. } => "rejected",
        CollectError::Fetch(_) => "fetch_error",
        CollectError::Extract(_) => "extract_error",
        CollectError::Youtube(_) => "youtube_error",
        CollectError::Summary(_) => "llm_error",
        CollectError::Io(_) => "io_error",
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectResult {
    pub url: String,
    pub file: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchCollectResult {
    pub summary: BatchCollectSummary,
    pub items: Vec<CollectItemResult>,
}

impl BatchCollectResult {
    pub fn has_failures(&self) -> bool {
        self.summary.failed > 0
    }

    pub fn failure_message(&self) -> String {
        format!(
            "{} of {} collect inputs failed",
            self.summary.failed, self.summary.total
        )
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BatchCollectSummary {
    pub total: usize,
    pub collected: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollectItemResult {
    pub input: String,
    pub status: CollectItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CollectItemStatus {
    Collected,
    Skipped,
    Failed,
}

#[derive(Debug, Clone)]
pub enum CollectOutput {
    Single(CollectResult),
    Batch(BatchCollectResult),
}

#[derive(Debug, Clone)]
enum ExpandedCollectInput {
    Url {
        input: String,
        url: String,
        from_file: bool,
    },
    Failure {
        item: CollectItemResult,
        from_file: bool,
    },
}

impl ExpandedCollectInput {
    fn is_from_file(&self) -> bool {
        match self {
            ExpandedCollectInput::Url { from_file, .. }
            | ExpandedCollectInput::Failure { from_file, .. } => *from_file,
        }
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

pub fn collect_inputs_with_collector<F>(
    inputs: Vec<String>,
    output_dir: &Path,
    mut collector: F,
) -> Result<CollectOutput, CollectError>
where
    F: FnMut(&str) -> Result<Document, CollectError>,
{
    let expanded = expand_collect_inputs(&inputs);
    let batch_mode = inputs.len() > 1 || expanded.iter().any(ExpandedCollectInput::is_from_file);

    if !batch_mode {
        let Some(ExpandedCollectInput::Url { url, .. }) = expanded.first() else {
            return Ok(CollectOutput::Batch(collect_batch(
                expanded,
                output_dir,
                &mut collector,
            )));
        };
        let page = collector(url)?;
        return Ok(CollectOutput::Single(collect_result_from_document(
            output_dir, page,
        )));
    }

    Ok(CollectOutput::Batch(collect_batch(
        expanded,
        output_dir,
        &mut collector,
    )))
}

fn expand_collect_inputs(inputs: &[String]) -> Vec<ExpandedCollectInput> {
    inputs
        .iter()
        .flat_map(|input| expand_collect_input(input))
        .collect()
}

fn expand_collect_input(input: &str) -> Vec<ExpandedCollectInput> {
    if !is_url_list_file(input) {
        return vec![ExpandedCollectInput::Url {
            input: input.to_string(),
            url: input.to_string(),
            from_file: false,
        }];
    }

    let contents = match fs::read_to_string(input) {
        Ok(contents) => contents,
        Err(error) => {
            return vec![ExpandedCollectInput::Failure {
                item: collect_failure_item(
                    input,
                    None,
                    "url_list_read_error",
                    format!("failed to read URL list: {error}"),
                ),
                from_file: true,
            }]
        }
    };

    let mut urls = Vec::new();
    for (line_index, line) in contents.lines().enumerate() {
        let url = line.trim();
        if url.is_empty() {
            continue;
        }
        urls.push(ExpandedCollectInput::Url {
            input: format!("{}:{}", input, line_index + 1),
            url: url.to_string(),
            from_file: true,
        });
    }

    if urls.is_empty() {
        urls.push(ExpandedCollectInput::Failure {
            item: collect_failure_item(
                input,
                None,
                "empty_url_list",
                "URL list file contains no URLs",
            ),
            from_file: true,
        });
    }

    urls
}

fn is_url_list_file(input: &str) -> bool {
    let path = Path::new(input);
    let has_txt_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"));

    has_txt_extension && (path.is_file() || !input.contains("://"))
}

fn collect_batch<F>(
    expanded: Vec<ExpandedCollectInput>,
    output_dir: &Path,
    collector: &mut F,
) -> BatchCollectResult
where
    F: FnMut(&str) -> Result<Document, CollectError>,
{
    let mut items = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();

    for input in expanded {
        let (input, url) = match input {
            ExpandedCollectInput::Url { input, url, .. } => (input, url),
            ExpandedCollectInput::Failure { item, .. } => {
                items.push(item);
                continue;
            }
        };

        if let Some(first_input) = seen.get(&url) {
            items.push(collect_skipped_item(
                &input,
                &url,
                "duplicate_input",
                format!("duplicate input URL first listed at {first_input}"),
                None,
            ));
            continue;
        }
        seen.insert(url.clone(), input.clone());

        match duplicate_file(&url, output_dir) {
            Ok(Some(existing_file)) => {
                items.push(collect_skipped_item(
                    &input,
                    &url,
                    "duplicate_url",
                    format!("already collected → {existing_file}"),
                    Some(existing_file),
                ));
                continue;
            }
            Ok(None) => {}
            Err(error) => {
                items.push(collect_item_from_error(&input, &url, error));
                continue;
            }
        }

        match collector(&url) {
            Ok(page) => items.push(collect_success_item(
                &input,
                collect_result_from_document(output_dir, page),
            )),
            Err(CollectError::DuplicateUrl { existing_file }) => items.push(collect_skipped_item(
                &input,
                &url,
                "duplicate_url",
                format!("already collected → {existing_file}"),
                Some(existing_file),
            )),
            Err(error) => items.push(collect_item_from_error(&input, &url, error)),
        }
    }

    let summary = summarize_collect_items(&items);
    BatchCollectResult { summary, items }
}

fn collect_result_from_document(output_dir: &Path, page: Document) -> CollectResult {
    let path = output_dir.join(&page.filename);
    CollectResult {
        url: page.url,
        file: page.filename,
        path: path.display().to_string(),
    }
}

fn collect_success_item(input: &str, result: CollectResult) -> CollectItemResult {
    CollectItemResult {
        input: input.to_string(),
        status: CollectItemStatus::Collected,
        url: Some(result.url),
        file: Some(result.file),
        path: Some(result.path),
        code: None,
        message: None,
        existing_file: None,
        reason: None,
    }
}

fn collect_skipped_item(
    input: &str,
    url: &str,
    code: &str,
    message: String,
    existing_file: Option<String>,
) -> CollectItemResult {
    CollectItemResult {
        input: input.to_string(),
        status: CollectItemStatus::Skipped,
        url: Some(url.to_string()),
        file: None,
        path: None,
        code: Some(code.to_string()),
        message: Some(message),
        existing_file,
        reason: None,
    }
}

fn collect_failure_item(
    input: &str,
    url: Option<&str>,
    code: &str,
    message: impl Into<String>,
) -> CollectItemResult {
    CollectItemResult {
        input: input.to_string(),
        status: CollectItemStatus::Failed,
        url: url.map(str::to_string),
        file: None,
        path: None,
        code: Some(code.to_string()),
        message: Some(message.into()),
        existing_file: None,
        reason: None,
    }
}

fn collect_item_from_error(input: &str, url: &str, error: CollectError) -> CollectItemResult {
    let mut item = collect_failure_item(input, Some(url), error_code(&error), error.to_string());
    match error {
        CollectError::DuplicateUrl { existing_file } => {
            item.status = CollectItemStatus::Skipped;
            item.existing_file = Some(existing_file);
        }
        CollectError::Rejected { reason, .. } => {
            item.reason = Some(reason.to_string());
        }
        _ => {}
    }
    item
}

fn summarize_collect_items(items: &[CollectItemResult]) -> BatchCollectSummary {
    BatchCollectSummary {
        total: items.len(),
        collected: items
            .iter()
            .filter(|item| item.status == CollectItemStatus::Collected)
            .count(),
        skipped: items
            .iter()
            .filter(|item| item.status == CollectItemStatus::Skipped)
            .count(),
        failed: items
            .iter()
            .filter(|item| item.status == CollectItemStatus::Failed)
            .count(),
    }
}

pub fn duplicate_file(url: &str, output_dir: &Path) -> Result<Option<String>, CollectError> {
    let index_path = tree::index_path(output_dir);
    let entries = index::read_index(&index_path)?;
    Ok(index::is_duplicate(&entries, url).map(|existing| existing.file.clone()))
}

fn ensure_not_duplicate(url: &str, output_dir: &Path) -> Result<(), CollectError> {
    if let Some(existing_file) = duplicate_file(url, output_dir)? {
        return Err(CollectError::DuplicateUrl { existing_file });
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
    let index_path = tree::index_path(output_dir);
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
