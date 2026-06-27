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

use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;
use std::thread;

use crate::adapters::youtube::{self, YoutubeError, YoutubeUrlMatch};
use crate::cli::json::JsonError;
use crate::domain::manifest::{self, LeafRecord, Manifest, TreeMeta};
use crate::domain::slug::Slug;
use crate::domain::{leaf, slug, Timestamp};
use crate::engine::auth;
use crate::engine::pending::{self, OpKind, PendingWrite};
use crate::engine::quality::RejectReason;
use crate::engine::{extract, fetch, quality, summary};
use serde_json::json;

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
    Manifest(manifest::ManifestError),
    Pending(pending::PendingError),
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
            CollectError::Manifest(e) => write!(f, "{}", e),
            CollectError::Pending(e) => write!(f, "{}", e),
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

impl From<manifest::ManifestError> for CollectError {
    fn from(e: manifest::ManifestError) -> Self {
        CollectError::Manifest(e)
    }
}

impl From<pending::PendingError> for CollectError {
    fn from(e: pending::PendingError) -> Self {
        CollectError::Pending(e)
    }
}

pub fn error_code(error: &CollectError) -> &'static str {
    match error {
        CollectError::DuplicateUrl { .. } => "duplicate_url",
        CollectError::Rejected { .. } => "rejected",
        CollectError::Fetch(_) => "fetch_error",
        CollectError::Extract(_) => "extract_error",
        CollectError::Youtube(_) => "youtube_error",
        CollectError::Io(_) => "io_error",
        CollectError::Manifest(_) => "manifest_error",
        CollectError::Pending(pending::PendingError::Busy { .. }) => "tree_busy",
        CollectError::Pending(_) => "pending_error",
    }
}

impl CollectError {
    pub fn json_error(&self) -> JsonError {
        match self {
            CollectError::DuplicateUrl { existing_file } => JsonError::with_details(
                error_code(self),
                self.to_string(),
                json!({ "existing_file": existing_file }),
            ),
            CollectError::Rejected { url, reason } => JsonError::with_details(
                error_code(self),
                self.to_string(),
                json!({ "url": url, "reason": reason.to_string() }),
            ),
            _ => JsonError::new(error_code(self), self.to_string()),
        }
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

// ── data for parallel batch compute ──────────────────────────────────────────

/// Output of the compute phase: everything needed to write a leaf, but no
/// disk I/O performed yet. Safe to produce from multiple threads concurrently.
struct ComputedLeaf {
    url: String,
    title: Option<String>,
    body_markdown: String,
    summary_text: String,
}

// ── pipeline ─────────────────────────────────────────────────────────────────

/// Compute-only: fetch, extract, quality-check, and summarize a URL.
/// Returns the data needed to write a leaf without touching the manifest or
/// output directory. Safe to call from multiple threads.
fn compute_leaf_url(
    url: &str,
    model: &str,
    provider: crate::engine::llm::Provider,
) -> Result<ComputedLeaf, CollectError> {
    // YouTube path — fetch transcript, summarize, return.
    match youtube::classify_url(url) {
        YoutubeUrlMatch::Supported(supported) => {
            let transcript = youtube::collect_transcript(&supported)?;
            let summary_text = generate_summary_with_model(
                &transcript.body_markdown,
                Some(&transcript.title),
                model,
                provider,
            );
            return Ok(ComputedLeaf {
                url: transcript.url,
                title: Some(transcript.title),
                body_markdown: transcript.body_markdown,
                summary_text,
            });
        }
        YoutubeUrlMatch::Unsupported { url, reason } => {
            return Err(YoutubeError::UnsupportedUrl { url, reason }.into());
        }
        YoutubeUrlMatch::NotYoutube => {}
    }

    // Fetch, classify HTTP errors, extract, quality-check, summarize.
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

    if let Some(reason) = quality::classify_html(&fetched.html) {
        return Err(CollectError::Rejected {
            url: fetched.url.clone(),
            reason,
        });
    }

    let content = extract::extract_content(&fetched.html)?;

    if let Some(reason) =
        quality::classify_extracted(content.title.as_deref(), &content.body_markdown)
    {
        return Err(CollectError::Rejected {
            url: fetched.url.clone(),
            reason,
        });
    }

    let summary_text = generate_summary_with_model(
        &content.body_markdown,
        content.title.as_deref(),
        model,
        provider,
    );

    Ok(ComputedLeaf {
        url: fetched.url,
        title: content.title,
        body_markdown: content.body_markdown,
        summary_text,
    })
}

pub fn collect_url_with_model(
    url: &str,
    output_dir: &Path,
    model: &str,
    provider: crate::engine::llm::Provider,
) -> Result<Document, CollectError> {
    recover_pending_if_needed(output_dir)?;

    match youtube::classify_url(url) {
        YoutubeUrlMatch::Supported(supported) => {
            let transcript = youtube::collect_transcript(&supported)?;
            return write_new_document_with_model(
                &transcript.url,
                Some(&transcript.title),
                &transcript.body_markdown,
                output_dir,
                model,
                provider,
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
    collect_html_with_model(&fetched.url, &fetched.html, output_dir, model, provider)
}

pub fn collect_html_with_model(
    url: &str,
    html: &str,
    output_dir: &Path,
    model: &str,
    provider: crate::engine::llm::Provider,
) -> Result<Document, CollectError> {
    collect_html_with_summarizer(url, html, output_dir, |body, title| {
        generate_summary_with_model(body, title, model, provider)
    })
}

pub fn collect_html_with_summarizer<F>(
    url: &str,
    html: &str,
    output_dir: &Path,
    summarize: F,
) -> Result<Document, CollectError>
where
    F: FnOnce(&str, Option<&str>) -> String,
{
    if let Some(reason) = quality::classify_html(html) {
        return Err(CollectError::Rejected {
            url: url.to_string(),
            reason,
        });
    }

    let content = extract::extract_content(html)?;

    if let Some(reason) =
        quality::classify_extracted(content.title.as_deref(), &content.body_markdown)
    {
        return Err(CollectError::Rejected {
            url: url.to_string(),
            reason,
        });
    }

    let summary_text = summarize(&content.body_markdown, content.title.as_deref());
    write_new_document_with_summary_result(
        url,
        content.title.as_deref(),
        &content.body_markdown,
        output_dir,
        summary_text,
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
    let batch_mode = inputs.len() > 1
        || expanded.iter().any(|input| {
            matches!(
                input,
                ExpandedCollectInput::Url {
                    from_file: true,
                    ..
                } | ExpandedCollectInput::Failure {
                    from_file: true,
                    ..
                }
            )
        });

    if !batch_mode {
        let Some(ExpandedCollectInput::Url { url, .. }) = expanded.first() else {
            return Ok(CollectOutput::Batch(collect_batch(
                expanded,
                output_dir,
                &mut collector,
            )));
        };
        ensure_not_duplicate(url, output_dir)?;
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
    if !has_txt_extension {
        return false;
    }
    // A URL containing :// is never a local URL list file.
    if input.contains("://") {
        return false;
    }
    // Bare domains ending in .txt (e.g. example.com/urls.txt) should not be
    // mistaken for local .txt files. If the part before the first '/' looks
    // like a hostname (contains a dot), treat the input as a URL.
    let before_slash = input.split('/').next().unwrap_or(input);
    if before_slash.contains('.') {
        return false;
    }
    true
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
    let manifest_path = output_dir.join(".bo").join("manifest.json");
    let manifest = match manifest::read(&manifest_path) {
        Ok(m) => m,
        Err(manifest::ManifestError::TreeNotInitialized) => return Ok(None),
        Err(e) => return Err(CollectError::Manifest(e)),
    };
    Ok(manifest
        .leaves
        .iter()
        .find(|l| l.url.as_str() == url)
        .map(|l| l.file.clone()))
}

fn recover_pending_if_needed(output_dir: &Path) -> Result<(), CollectError> {
    if let Some(report) = pending::recover_or_refuse(output_dir)? {
        eprintln!(
            "recovered {} changes from interrupted {}",
            report.changes, report.op
        );
    }
    Ok(())
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
    provider: crate::engine::llm::Provider,
) -> Result<Document, CollectError> {
    let summary_text = generate_summary_with_model(body_markdown, title, model, provider);
    write_new_document_with_summary_result(url, title, body_markdown, output_dir, summary_text)
}

// ponytail: one runtime per process, separate runtimes if we ever need concurrent batches
fn summary_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build tokio runtime")
    })
}

fn generate_summary_with_model(
    body: &str,
    title: Option<&str>,
    model: &str,
    provider: crate::engine::llm::Provider,
) -> String {
    let api_key = match auth::resolve_api_key(provider) {
        Ok(key) => key,
        Err(_) => return summary::generate_fallback(body),
    };
    let provider = crate::engine::llm::create_provider(provider, &api_key);
    summary_runtime().block_on(summary::generate_llm_or_fallback(
        body,
        title,
        provider.as_ref(),
        model,
        summary::SUMMARY_LLM_POLICY,
    ))
}

fn write_new_document_with_summary_result(
    url: &str,
    title: Option<&str>,
    body_markdown: &str,
    output_dir: &Path,
    summary_text: String,
) -> Result<Document, CollectError> {
    recover_pending_if_needed(output_dir)?;

    let title_ref = title.unwrap_or("");
    let base_slug = Slug::generate(title_ref, url);
    let filename = slug::resolve_slug(&base_slug, url, output_dir);
    let now = Timestamp::now();
    let domain_url = url.to_string();
    let domain_title = title.map(str::to_string);
    let leaf_file = format!("{}.md", filename);
    let summary_field = if summary_text.is_empty() {
        None
    } else {
        Some(summary_text.as_str())
    };
    let leaf_content = leaf::format_content(
        domain_title.as_ref(),
        &domain_url,
        &now,
        body_markdown,
        summary_field,
    );
    let leaf_write = PendingWrite {
        path: leaf_file.clone(),
        content_hash: pending::content_hash(leaf_content.as_bytes()),
    };

    let mut manifest = load_or_bootstrap_manifest(output_dir, &now)?;
    if let Some(existing) = manifest.leaves.iter().find(|leaf| leaf.url.as_str() == url) {
        return Err(CollectError::DuplicateUrl {
            existing_file: existing.file.clone(),
        });
    }
    manifest.leaves.push(LeafRecord {
        slug: filename.clone(),
        file: leaf_file.clone(),
        title: title.unwrap_or_default().to_string(),
        url: domain_url,
        collected_at: now,
        summary: if summary_text.is_empty() {
            None
        } else {
            Some(summary_text.clone())
        },
    });

    commit_manifest_and_writes(
        output_dir,
        OpKind::Collect {
            url: url.to_string(),
        },
        &manifest,
        &[(&leaf_write, leaf_content.as_bytes())],
        &[],
    )?;

    Ok(Document {
        url: url.to_string(),
        filename: leaf_file,
    })
}

/// Load the manifest from disk, or return an empty one if the tree is freshly seeded.
fn load_or_bootstrap_manifest(
    output_dir: &Path,
    now: &Timestamp,
) -> Result<Manifest, CollectError> {
    let manifest_path = output_dir.join(".bo").join("manifest.json");
    match manifest::read(&manifest_path) {
        Ok(m) => Ok(m),
        Err(manifest::ManifestError::TreeNotInitialized) => Ok(Manifest {
            tree: TreeMeta {
                name: output_dir
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "unnamed".to_string()),
                created_at: now.clone(),
                last_compiled_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        }),
        Err(e) => Err(CollectError::Manifest(e)),
    }
}

/// Atomically commit a manifest update with staged writes and deletes.
///
/// Writes the pending operation, stages content, writes the manifest, then
/// applies writes/deletes and clears the pending file. The entire sequence
/// is guarded by the pending lock so a crash mid-commit is recoverable.
fn commit_manifest_and_writes(
    output_dir: &Path,
    op: OpKind,
    manifest: &Manifest,
    staged: &[(&PendingWrite, &[u8])],
    deletes: &[String],
) -> Result<(), CollectError> {
    let writes: Vec<PendingWrite> = staged.iter().map(|(pw, _)| (*pw).clone()).collect();
    let operation = pending::new_operation(output_dir, op, writes.clone(), deletes.to_vec())?;
    let pending_path = pending::pending_path(output_dir);
    pending::write(&pending_path, &operation)?;
    for (pw, bytes) in staged {
        pending::write_staged(output_dir, pw, bytes)?;
    }
    let manifest_path = output_dir.join(".bo").join("manifest.json");
    manifest::write(&manifest_path, manifest)?;
    pending::apply_writes(output_dir, &writes)?;
    pending::apply_deletes(output_dir, deletes)?;
    pending::clear(&pending_path)?;
    Ok(())
}

// ── parallel batch collect ───────────────────────────────────────────────────

/// Collect a batch of URLs using parallel fetch+extract+summarize, then
/// commit all writes in a single manifest operation.
///
/// Falls back to sequential `collect_batch` behavior when there is only one
/// URL to collect (single thread overhead isn't worth it).
pub fn collect_batch_parallel(
    inputs: Vec<String>,
    output_dir: &Path,
    model: &str,
    provider: crate::engine::llm::Provider,
) -> Result<BatchCollectResult, CollectError> {
    recover_pending_if_needed(output_dir)?;
    let expanded = expand_collect_inputs(&inputs);

    // ── phase 1: dedup (sequential) ─────────────────────────────────────
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut items: Vec<CollectItemResult> = Vec::new();
    let mut to_compute: Vec<(String, String)> = Vec::new();

    for input in expanded {
        let (input_label, url) = match input {
            ExpandedCollectInput::Url { input, url, .. } => (input, url),
            ExpandedCollectInput::Failure { item, .. } => {
                items.push(item);
                continue;
            }
        };

        if let Some(first_input) = seen.get(&url) {
            items.push(collect_skipped_item(
                &input_label,
                &url,
                "duplicate_input",
                format!("duplicate input URL first listed at {first_input}"),
                None,
            ));
            continue;
        }
        seen.insert(url.clone(), input_label.clone());

        match duplicate_file(&url, output_dir) {
            Ok(Some(existing_file)) => {
                items.push(collect_skipped_item(
                    &input_label,
                    &url,
                    "duplicate_url",
                    format!("already collected → {existing_file}"),
                    Some(existing_file),
                ));
                continue;
            }
            Ok(None) => {}
            Err(error) => {
                items.push(collect_item_from_error(&input_label, &url, error));
                continue;
            }
        }

        to_compute.push((input_label, url));
    }

    // ── phase 2: parallel compute ────────────────────────────────────────
    let compute_results: Vec<(String, String, Result<ComputedLeaf, CollectError>)> = if to_compute
        .is_empty()
    {
        Vec::new()
    } else {
        // Chunk to limit concurrent threads — each thread is I/O-bound.
        const CHUNK_SIZE: usize = 20;
        let mut results = Vec::with_capacity(to_compute.len());
        for chunk in to_compute.chunks(CHUNK_SIZE) {
            let handles: Vec<(String, String, thread::JoinHandle<_>)> = chunk
                .iter()
                .map(|(input, url)| {
                    let input = input.clone();
                    let url = url.clone();
                    let model = model.to_string();
                    let url_for_thread = url.clone();
                    let handle =
                        thread::spawn(move || compute_leaf_url(&url_for_thread, &model, provider));
                    (input, url, handle)
                })
                .collect();
            for (input, url, handle) in handles {
                match handle.join() {
                    Ok(compute_result) => results.push((input, url, compute_result)),
                    Err(panic) => {
                        let msg = if let Some(s) = panic.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = panic.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            format!("{panic:?}")
                        };
                        results.push((
                            input,
                            url.clone(),
                            Err(CollectError::Fetch(fetch::FetchError::Network(format!(
                                "compute thread panicked for {url}: {msg}"
                            )))),
                        ));
                    }
                }
            }
        }
        results
    };

    // ── phase 3: sequential commit ───────────────────────────────────────
    let now = Timestamp::now();
    let mut manifest = load_or_bootstrap_manifest(output_dir, &now)?;
    let mut staged: Vec<(PendingWrite, Vec<u8>)> = Vec::new();

    for (input_label, url, result) in compute_results {
        match result {
            Ok(computed) => {
                let base_slug =
                    Slug::generate(computed.title.as_deref().unwrap_or(""), &computed.url);
                let filename = slug::resolve_slug(&base_slug, &computed.url, output_dir);
                let leaf_file = format!("{}.md", filename);
                let summary_field = if computed.summary_text.is_empty() {
                    None
                } else {
                    Some(computed.summary_text.as_str())
                };
                let leaf_content = leaf::format_content(
                    computed.title.as_ref(),
                    &computed.url,
                    &now,
                    &computed.body_markdown,
                    summary_field,
                );
                let leaf_bytes = leaf_content.into_bytes();
                let leaf_write = PendingWrite {
                    path: leaf_file.clone(),
                    content_hash: pending::content_hash(&leaf_bytes),
                };

                // Check duplicate against in-memory manifest (catches same-batch duplicates).
                if manifest
                    .leaves
                    .iter()
                    .any(|l| l.url.as_str() == computed.url)
                {
                    items.push(collect_skipped_item(
                        &input_label,
                        &computed.url,
                        "duplicate_url",
                        format!("already collected → {leaf_file}"),
                        Some(leaf_file),
                    ));
                    continue;
                }

                manifest.leaves.push(LeafRecord {
                    slug: filename.clone(),
                    file: leaf_file.clone(),
                    title: computed.title.unwrap_or_default(),
                    url: computed.url.clone(),
                    collected_at: now.clone(),
                    summary: if computed.summary_text.is_empty() {
                        None
                    } else {
                        Some(computed.summary_text)
                    },
                });

                let result = CollectResult {
                    url: computed.url,
                    file: leaf_file,
                    path: output_dir.join(&leaf_write.path).display().to_string(),
                };
                items.push(collect_success_item(&input_label, result));
                staged.push((leaf_write, leaf_bytes));
            }
            Err(e) => {
                items.push(collect_item_from_error(&input_label, &url, e));
            }
        }
    }

    // Single commit for all writes.
    if !staged.is_empty() {
        let staged_refs: Vec<(&PendingWrite, &[u8])> =
            staged.iter().map(|(pw, b)| (pw, b.as_slice())).collect();
        commit_manifest_and_writes(
            output_dir,
            OpKind::Collect {
                url: format!("batch of {} urls", staged_refs.len()),
            },
            &manifest,
            &staged_refs,
            &[],
        )?;
    }

    let summary = summarize_collect_items(&items);
    Ok(BatchCollectResult { summary, items })
}

#[cfg(test)]
#[path = "../tests/cli_collect_tests.rs"]
mod tests;

// ── human rendering ──────────────────────────────────────────────────────────

pub fn render_human<W: Write>(result: &CollectResult, stdout: &mut W) -> std::io::Result<()> {
    writeln!(stdout, "✓ collected: {} → {}", result.url, result.file)
}

pub fn render_batch_human<W: Write>(
    result: &BatchCollectResult,
    stdout: &mut W,
) -> std::io::Result<()> {
    for item in &result.items {
        let label = item.url.as_deref().unwrap_or(&item.input);
        match item.status {
            CollectItemStatus::Collected => writeln!(
                stdout,
                "✓ collected: {} → {}",
                label,
                item.file.as_deref().unwrap_or("")
            )?,
            CollectItemStatus::Skipped => writeln!(
                stdout,
                "↷ skipped: {} ({})",
                label,
                item.message.as_deref().unwrap_or("skipped")
            )?,
            CollectItemStatus::Failed => writeln!(
                stdout,
                "✗ failed: {} ({})",
                label,
                item.message.as_deref().unwrap_or("failed")
            )?,
        }
    }

    writeln!(
        stdout,
        "collect summary: {} collected, {} skipped, {} failed",
        result.summary.collected, result.summary.skipped, result.summary.failed
    )
}
