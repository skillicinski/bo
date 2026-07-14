// bo collect — the collect pipeline.
//
// Single URLs, URL lists, multi-URL, and local notes all route through ONE
// pipeline: expand → dedup → compute → single-atomic-commit. The summary
// provider is resolved once per invocation and shared across worker threads.
// `CollectOutput::Single` vs `Batch` is output policy only — a single bare
// URL returns the single-result contract; every other input shape returns
// a batch.
//
// Dependency direction: collect → adapters, fetch, quality, extract, leaf, slug, manifest, pending.

use serde::Serialize;
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::Arc;
use std::thread;

use crate::adapters::youtube::{self, YoutubeError, YoutubeUrlMatch};
use crate::cli::json::JsonError;
use crate::domain::manifest::{self, Manifest, TreeMeta};
use crate::domain::slug::Slug;
use crate::domain::Leaf;
use crate::domain::{leaf, slug, Timestamp, Title, Url};
use crate::engine::auth;
use crate::engine::pending::{self, OpKind, PendingWrite};
use crate::engine::quality::RejectReason;
use crate::engine::{extract, fetch, quality, summary};
use serde_json::json;

// ── types ────────────────────────────────────────────────────────────────────

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
    Note(NoteError),
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
            CollectError::Note(e) => write!(f, "{}", e),
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
        CollectError::Note(NoteError::Read { .. }) => "note_read_error",
        CollectError::Note(NoteError::Empty { .. }) => "empty_note",
        CollectError::Note(NoteError::MalformedFrontmatter { .. }) => "malformed_frontmatter",
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

// ── note errors ──────────────────────────────────────────────────────────────

/// Errors specific to collecting a local markdown note.
#[derive(Debug)]
pub enum NoteError {
    Read { path: String, error: std::io::Error },
    Empty { path: String },
    MalformedFrontmatter { path: String },
}

impl fmt::Display for NoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NoteError::Read { path, error } => write!(f, "failed to read note {}: {}", path, error),
            NoteError::Empty { path } => write!(f, "note {} has no content", path),
            NoteError::MalformedFrontmatter { path } => {
                write!(f, "note {} has malformed frontmatter", path)
            }
        }
    }
}

impl std::error::Error for NoteError {}

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
    Url { input: String, url: String },
    Note { input: String, path: String },
    Failure { item: CollectItemResult },
}

// ── SummaryProvider — resolve once per invocation ────────────────────────────

/// Resolved summary provider, shared across all worker threads. Auth and
/// provider construction happen once per collect invocation; failure falls
/// back to deterministic summaries.
type Summarize = Arc<dyn Fn(&str, Option<&str>) -> String + Send + Sync>;

#[derive(Clone)]
struct SummaryProvider {
    summarize: Summarize,
}

impl SummaryProvider {
    fn resolve(
        provider: crate::engine::llm::Provider,
        model: String,
        base_url: Option<&str>,
    ) -> Self {
        let resolved = (|| {
            let api_key = auth::resolve_api_key(provider).ok()?;
            crate::engine::llm::create_provider(provider, &api_key, base_url).ok()
        })();
        match resolved {
            Some(p) => {
                let p: Arc<dyn crate::engine::llm::LlmProvider> = Arc::from(p);
                Self {
                    summarize: Arc::new(move |body, title| {
                        crate::engine::llm::blocking_runtime().block_on(
                            summary::generate_llm_or_fallback(
                                body,
                                title,
                                p.as_ref(),
                                &model,
                                summary::SUMMARY_LLM_POLICY,
                            ),
                        )
                    }),
                }
            }
            None => Self {
                summarize: Arc::new(|body, _| summary::generate_fallback(body)),
            },
        }
    }

    #[cfg(test)]
    fn fallback() -> Self {
        Self {
            summarize: Arc::new(|body, _| summary::generate_fallback(body)),
        }
    }

    fn summarize(&self, body: &str, title: Option<&str>) -> String {
        (self.summarize)(body, title)
    }
}

// ── outcome ──────────────────────────────────────────────────────────────────

/// Internal result from the pipeline, before shaping into output variants.
enum Outcome {
    Collected {
        input: String,
        result: CollectResult,
    },
    Errored {
        input: String,
        url: String,
        error: CollectError,
    },
    Item(CollectItemResult),
}

// ── journal ───────────────────────────────────────────────────────────────────

#[derive(Serialize)]
struct CollectJournalItem<'a> {
    input: &'a str,
    status: CollectItemStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: &'a Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: &'a Option<String>,
}

impl<'a> From<&'a CollectItemResult> for CollectJournalItem<'a> {
    fn from(item: &'a CollectItemResult) -> Self {
        Self {
            input: &item.input,
            status: item.status,
            url: &item.url,
            file: &item.file,
            code: &item.code,
            message: &item.message,
        }
    }
}

#[derive(Serialize)]
struct CollectJournalPayload<'a> {
    items: Vec<CollectJournalItem<'a>>,
}

/// Record a collect operation in the tree's journal. Best-effort: a journal
/// failure never fails the command. `model` is included only when at least one
/// real (non-note) URL was processed — notes collect no LLM summary.
pub fn journal(tree_dir: &Path, model: &str, items: &[CollectItemResult]) {
    let involved = items
        .iter()
        .any(|i| i.url.as_deref().is_some_and(|u| !u.starts_with("bo://")));
    let payload = CollectJournalPayload {
        items: items.iter().map(CollectJournalItem::from).collect(),
    };
    crate::engine::journal::append_payload(
        tree_dir,
        crate::engine::journal::Op::Collect,
        if involved {
            Some(model.to_string())
        } else {
            None
        },
        &payload,
    );
}

// ── data for parallel batch compute ──────────────────────────────────────────

/// Output of the compute phase: everything needed to write a leaf, but no
/// disk I/O performed yet. Safe to produce from multiple threads concurrently.
#[derive(Debug)]
struct ComputedLeaf {
    url: String,
    title: Option<String>,
    body_markdown: String,
    summary_text: String,
    /// Frontmatter-strip warning for notes; `None` for fetched URLs.
    note_warning: Option<String>,
}

// ── pipeline ─────────────────────────────────────────────────────────────────

/// Compute-only: fetch, extract, quality-check, and summarize a URL.
/// Returns the data needed to write a leaf without touching the manifest or
/// output directory. Safe to call from multiple threads.
fn compute_leaf(url: &str, summary: &SummaryProvider) -> Result<ComputedLeaf, CollectError> {
    // YouTube path — fetch transcript, summarize, return.
    match youtube::classify_url(url) {
        YoutubeUrlMatch::Supported(supported) => {
            let transcript = youtube::collect_transcript(&supported)?;
            let summary_text =
                summary.summarize(&transcript.body_markdown, Some(&transcript.title));
            return Ok(ComputedLeaf {
                url: transcript.url,
                title: Some(transcript.title),
                body_markdown: transcript.body_markdown,
                summary_text,
                note_warning: None,
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

    compute_leaf_from_html(&fetched.url, &fetched.html, |b, t| summary.summarize(b, t))
}

/// Test seam: the HTML core of `compute_leaf` without network I/O.
/// Injects pre-fetched HTML and a custom summarize closure.
fn compute_leaf_from_html<F>(
    url: &str,
    html: &str,
    summarize: F,
) -> Result<ComputedLeaf, CollectError>
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

    Ok(ComputedLeaf {
        url: url.to_string(),
        title: content.title,
        body_markdown: content.body_markdown,
        summary_text,
        note_warning: None,
    })
}

/// Compute a note from a local `.md` file: read, strip user frontmatter,
/// extract a title from the leading H1, and derive a content-addressed source
/// URL. No fetch, no extract, no summary — notes store `summary: None`.
///
/// # ponytail: `bo://note/<sha256[:16]>` is a documented overload of `url` as
/// source-id for source-less leaves. Upgrade to `Option<Url>` in the v0.1.0
/// restructure if notes prove out.
fn compute_leaf_note(path: &str) -> Result<ComputedLeaf, CollectError> {
    let raw = fs::read_to_string(path).map_err(|error| {
        CollectError::Note(NoteError::Read {
            path: path.to_string(),
            error,
        })
    })?;
    let (body_after_frontmatter, note_warning) = strip_user_frontmatter(path, &raw)?;
    if body_after_frontmatter.trim().is_empty() {
        return Err(CollectError::Note(NoteError::Empty {
            path: path.to_string(),
        }));
    }
    // Hash the frontmatter-stripped body (including the H1 line) so identical
    // notes dedup and edits mint a fresh leaf. sha256[:16] = 64 bits.
    let url = format!(
        "bo://note/{}",
        &pending::content_hash(body_after_frontmatter.as_bytes())[..16]
    );
    let (title, body_markdown) = extract_note_title(&body_after_frontmatter);
    Ok(ComputedLeaf {
        url,
        title,
        body_markdown,
        summary_text: String::new(),
        note_warning,
    })
}

/// Strip a leading YAML frontmatter block from a note. Returns the body and
/// an optional warning when non-empty user frontmatter was removed — leaf
/// frontmatter is bo-owned, so the shaping is always visible.
fn strip_user_frontmatter(path: &str, raw: &str) -> Result<(String, Option<String>), CollectError> {
    use crate::domain::frontmatter::{parse, FrontmatterError};
    match parse(raw) {
        Ok((mapping, body)) => {
            let warning = if mapping.is_empty() {
                None
            } else {
                Some(format!("stripped user frontmatter from {}", path))
            };
            Ok((body, warning))
        }
        Err(FrontmatterError::Missing) => Ok((raw.to_string(), None)),
        Err(_) => Err(CollectError::Note(NoteError::MalformedFrontmatter {
            path: path.to_string(),
        })),
    }
}

/// Pull a title from a leading `# ` heading and strip it from the body,
/// mirroring fetched-page handling (`format_content` re-prepends `# title`).
fn extract_note_title(body: &str) -> (Option<String>, String) {
    let after_blank_lines = body.trim_start_matches('\n');
    let Some(rest) = after_blank_lines.strip_prefix("# ") else {
        return (None, body.to_string());
    };
    let (heading, remainder) = match rest.find('\n') {
        Some(idx) => (
            &rest[..idx],
            rest[idx + 1..].trim_start_matches('\n').to_string(),
        ),
        None => (rest, String::new()),
    };
    let title = heading.trim();
    if title.is_empty() {
        return (None, body.to_string());
    }
    (Some(title.to_string()), remainder)
}

// ── input expansion ──────────────────────────────────────────────────────────

fn expand_collect_inputs(inputs: &[String]) -> Vec<ExpandedCollectInput> {
    inputs
        .iter()
        .flat_map(|input| expand_collect_input(input))
        .collect()
}

fn expand_collect_input(input: &str) -> Vec<ExpandedCollectInput> {
    if is_local_note_file(input) {
        return vec![ExpandedCollectInput::Note {
            input: input.to_string(),
            path: input.to_string(),
        }];
    }
    if !is_url_list_file(input) {
        return vec![ExpandedCollectInput::Url {
            input: input.to_string(),
            url: input.to_string(),
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

/// A local markdown note: `.md` extension (case-insensitive), no URL scheme,
/// and the file exists on disk. Existence naturally excludes bare domains
/// and `https://.../x.md` URLs.
pub fn is_local_note_file(input: &str) -> bool {
    if input.contains("://") {
        return false;
    }
    let path = Path::new(input);
    let is_md = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    is_md && path.is_file()
}

/// Whether `inputs` selects the single-result output contract. Mirrors the
/// pre-unification routing: a lone argument that is not a URL list (`.txt`
/// without a scheme) and not an existing local note is collected as a single
/// URL, selecting `CollectOutput::Single`; every other input shape returns a
/// batch. The `.txt` suffix check is intentionally broader than
/// `is_url_list_file` so bare `urls.txt`-style arguments keep their prior
/// batch routing (where expansion then treats them as URLs).
pub fn is_single_bare_url(inputs: &[String]) -> bool {
    if inputs.len() != 1 {
        return false;
    }
    let input = &inputs[0];
    let is_url_list_like = input.ends_with(".txt") && !input.contains("://");
    !is_url_list_like && !is_local_note_file(input)
}

// ── item constructors ────────────────────────────────────────────────────────

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

fn collect_item_from_error(input: &str, url: &str, error: &CollectError) -> CollectItemResult {
    let mut item = collect_failure_item(input, Some(url), error_code(error), error.to_string());
    match error {
        CollectError::DuplicateUrl { existing_file } => {
            item.status = CollectItemStatus::Skipped;
            item.existing_file = Some(existing_file.clone());
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

// ── dedup helpers ────────────────────────────────────────────────────────────

pub fn duplicate_file(url: &str, output_dir: &Path) -> Result<Option<String>, CollectError> {
    let manifest_path = output_dir.join(".bo").join("manifest.json");
    let manifest = match crate::engine::manifest::read(&manifest_path) {
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

/// Recovery notices are stderr-bound diagnostics: the pipeline collects them,
/// the CLI renders post-run (#138 presentation-purity contract).
fn recover_pending_if_needed(
    output_dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<(), CollectError> {
    if let Some(report) = pending::recover_or_refuse(output_dir)? {
        warnings.push(format!(
            "recovered {} changes from interrupted {}",
            report.changes, report.op
        ));
    }
    Ok(())
}

/// Snapshot on-disk slug stems (filenames minus `.md`) so slug resolution
/// can avoid collisions without probing the filesystem per slug.
///
/// Existing leaf slug stems from disk, scanned from `leaf/` (leaves moved out of
/// the tree root in the per-entity layout).
///
/// # ponytail: read_dir failure = empty set, matches old .exists() false-on-error semantics
fn existing_slug_stems(output_dir: &Path) -> std::collections::HashSet<String> {
    let mut stems = std::collections::HashSet::new();
    let entries = match std::fs::read_dir(output_dir.join("leaf")) {
        Ok(entries) => entries,
        Err(_) => return stems,
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(stem) = name.strip_suffix(".md") {
            stems.insert(stem.to_string());
        }
    }
    stems
}

/// Load the manifest from disk, or return an empty one if the tree is freshly seeded.
fn load_or_bootstrap_manifest(
    output_dir: &Path,
    now: &Timestamp,
) -> Result<Manifest, CollectError> {
    let manifest_path = output_dir.join(".bo").join("manifest.json");
    match crate::engine::manifest::read(&manifest_path) {
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
    pending::commit_with_manifest(output_dir, op, manifest, staged, deletes)
        .map_err(|e| CollectError::Io(std::io::Error::other(e.to_string())))
}

// ── unified pipeline ─────────────────────────────────────────────────────────

/// Expand, dedup, compute, and single-atomic-commit. Returns outcomes so the
/// caller can shape them into `CollectOutput::Single` or `CollectOutput::Batch`.
fn run_pipeline<F>(
    inputs: Vec<String>,
    output_dir: &Path,
    warnings: &mut Vec<String>,
    compute: F,
) -> Result<Vec<Outcome>, CollectError>
where
    F: Fn(&str) -> Result<ComputedLeaf, CollectError> + Send + Sync + 'static,
{
    recover_pending_if_needed(output_dir, warnings)?;
    let expanded = expand_collect_inputs(&inputs);

    // ── phase 1: dedup (sequential) ─────────────────────────────────────
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut to_compute: Vec<(String, String)> = Vec::new();
    // Notes are computed inline (no fetch) and carried to phase 3 pre-computed.
    let mut precomputed_notes: Vec<(String, ComputedLeaf)> = Vec::new();

    for input in expanded {
        let (input_label, url) = match input {
            ExpandedCollectInput::Url { input, url, .. } => (input, url),
            ExpandedCollectInput::Failure { item, .. } => {
                outcomes.push(Outcome::Item(item));
                continue;
            }
            ExpandedCollectInput::Note { input, path } => {
                let computed = match compute_leaf_note(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        outcomes.push(Outcome::Errored {
                            input,
                            url: path.clone(),
                            error: e,
                        });
                        continue;
                    }
                };
                let note_url = computed.url.clone();
                if let Some(first_input) = seen.get(&note_url) {
                    outcomes.push(Outcome::Item(collect_skipped_item(
                        &input,
                        &note_url,
                        "duplicate_input",
                        format!("duplicate note first listed at {first_input}"),
                        None,
                    )));
                    continue;
                }
                seen.insert(note_url.clone(), input.clone());
                match duplicate_file(&note_url, output_dir) {
                    Ok(Some(existing_file)) => {
                        outcomes.push(Outcome::Errored {
                            input,
                            url: note_url,
                            error: CollectError::DuplicateUrl { existing_file },
                        });
                        continue;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        outcomes.push(Outcome::Errored {
                            input,
                            url: note_url,
                            error,
                        });
                        continue;
                    }
                }
                precomputed_notes.push((input, computed));
                continue;
            }
        };

        if let Some(first_input) = seen.get(&url) {
            outcomes.push(Outcome::Item(collect_skipped_item(
                &input_label,
                &url,
                "duplicate_input",
                format!("duplicate input URL first listed at {first_input}"),
                None,
            )));
            continue;
        }
        seen.insert(url.clone(), input_label.clone());

        match duplicate_file(&url, output_dir) {
            Ok(Some(existing_file)) => {
                outcomes.push(Outcome::Errored {
                    input: input_label,
                    url,
                    error: CollectError::DuplicateUrl { existing_file },
                });
                continue;
            }
            Ok(None) => {}
            Err(error) => {
                outcomes.push(Outcome::Errored {
                    input: input_label,
                    url,
                    error,
                });
                continue;
            }
        }

        to_compute.push((input_label, url));
    }

    // ── phase 2: parallel compute ────────────────────────────────────────
    let compute = Arc::new(compute);
    let mut compute_results: Vec<(String, String, Result<ComputedLeaf, CollectError>)> =
        if to_compute.is_empty() {
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
                        let compute = Arc::clone(&compute);
                        let url_for_thread = url.clone();
                        let handle = thread::spawn(move || compute(&url_for_thread));
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

    // Fold pre-computed notes into the same commit stream as fetched URLs.
    for (input, computed) in precomputed_notes {
        compute_results.push((input, computed.url.clone(), Ok(computed)));
    }

    // ── phase 3: sequential commit ───────────────────────────────────────
    let now = Timestamp::now();
    let mut manifest = load_or_bootstrap_manifest(output_dir, &now)?;
    let mut staged: Vec<(PendingWrite, Vec<u8>)> = Vec::new();
    // Track claimed slug stems so intra-batch collisions are resolved
    // before writes hit disk. Snapshot on-disk stems first, then track
    // newly-claimed stems inside the loop (see issues #92, #145).
    let mut used_slugs = existing_slug_stems(output_dir);

    for (input_label, url, result) in compute_results {
        match result {
            Ok(computed) => {
                if let Some(warning) = &computed.note_warning {
                    warnings.push(warning.clone());
                }
                // Same-batch duplicate: two inputs that fetched the same canonical
                // URL. Skip before reserving a slug, reporting the already-written
                // leaf's file rather than a freshly-resolved (and never-written) one.
                if let Some(existing) = manifest
                    .leaves
                    .iter()
                    .find(|l| l.url.as_str() == computed.url)
                {
                    outcomes.push(Outcome::Errored {
                        input: input_label,
                        url: computed.url,
                        error: CollectError::DuplicateUrl {
                            existing_file: existing.file.clone(),
                        },
                    });
                    continue;
                }
                let base_slug =
                    Slug::generate(computed.title.as_deref().unwrap_or(""), &computed.url);
                let filename = slug::resolve_slug(&base_slug, &computed.url, &mut used_slugs);
                let leaf_file = format!("leaf/{}.md", filename);
                let domain_url =
                    Url::parse(&computed.url).expect("URL already validated at fetch time");
                let domain_title = computed.title.as_deref().and_then(|t| Title::parse(t).ok());
                let leaf_content = leaf::format_content(
                    domain_title.as_ref(),
                    &domain_url,
                    &now,
                    &computed.body_markdown,
                );
                let leaf_bytes = leaf_content.into_bytes();
                let leaf_write = PendingWrite {
                    path: leaf_file.clone(),
                    content_hash: pending::content_hash(&leaf_bytes),
                };

                manifest.leaves.push(Leaf {
                    slug: filename.clone(),
                    file: leaf_file.clone(),
                    title: domain_title,
                    url: domain_url.clone(),
                    collected_at: now.clone(),
                    summary: if computed.summary_text.is_empty() {
                        None
                    } else {
                        Some(computed.summary_text)
                    },
                });

                let result = CollectResult {
                    url: domain_url.as_str().to_string(),
                    file: leaf_file,
                    path: output_dir.join(&leaf_write.path).display().to_string(),
                };
                outcomes.push(Outcome::Collected {
                    input: input_label,
                    result,
                });
                staged.push((leaf_write, leaf_bytes));
            }
            Err(e) => {
                outcomes.push(Outcome::Errored {
                    input: input_label,
                    url,
                    error: e,
                });
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

    Ok(outcomes)
}

// ── shapers ──────────────────────────────────────────────────────────────────

fn shape_item(o: &Outcome) -> CollectItemResult {
    match o {
        Outcome::Collected { input, result } => collect_success_item(input, result.clone()),
        Outcome::Errored { input, url, error } => collect_item_from_error(input, url, error),
        Outcome::Item(item) => item.clone(),
    }
}

fn shape_single(outcomes: Vec<Outcome>) -> Result<CollectResult, CollectError> {
    match outcomes.into_iter().next() {
        Some(Outcome::Collected { result, .. }) => Ok(result),
        Some(Outcome::Errored { error, .. }) => Err(error),
        Some(Outcome::Item(_)) => unreachable!("single bare URL cannot produce a synthetic item"),
        None => unreachable!("single input produces one outcome"),
    }
}

// ── public entry points ──────────────────────────────────────────────────────

/// The one collect pipeline. Routes single URLs, URL lists, multi-URL, and
/// local notes through expand→dedup→compute→single-atomic-commit. Resolves
/// the summary provider once per invocation.
pub fn collect(
    inputs: Vec<String>,
    output_dir: &Path,
    provider: crate::engine::llm::Provider,
    model: &str,
    base_url: Option<&str>,
    warnings: &mut Vec<String>,
) -> Result<CollectOutput, CollectError> {
    let summary = SummaryProvider::resolve(provider, model.to_string(), base_url);
    collect_with_compute(inputs, output_dir, model, warnings, move |url| {
        compute_leaf(url, &summary)
    })
}

/// Run the unified pipeline with an injected compute closure, then journal and
/// shape the output. `collect` resolves the provider once and delegates here;
/// tests inject a compute closure to drive the pipeline (and its journaling)
/// without network access.
fn collect_with_compute<F>(
    inputs: Vec<String>,
    output_dir: &Path,
    model: &str,
    warnings: &mut Vec<String>,
    compute: F,
) -> Result<CollectOutput, CollectError>
where
    F: Fn(&str) -> Result<ComputedLeaf, CollectError> + Send + Sync + 'static,
{
    let single = is_single_bare_url(&inputs);
    let outcomes = run_pipeline(inputs, output_dir, warnings, compute)?;
    let items: Vec<CollectItemResult> = outcomes.iter().map(shape_item).collect();
    if single {
        // A single bare URL propagates its raw error (duplicate/rejected/fetch)
        // and, per the single-result contract, is not journaled on failure.
        match shape_single(outcomes) {
            Ok(result) => {
                journal(output_dir, model, &items);
                Ok(CollectOutput::Single(result))
            }
            Err(error) => Err(error),
        }
    } else {
        journal(output_dir, model, &items);
        Ok(CollectOutput::Batch(BatchCollectResult {
            summary: summarize_collect_items(&items),
            items,
        }))
    }
}

/// Test seam: the same pipeline as `collect` but with an injected compute
/// closure. Does NOT journal — tests do not check journaling.
#[cfg(test)]
fn collect_batch_parallel_with_compute<F>(
    inputs: Vec<String>,
    output_dir: &Path,
    warnings: &mut Vec<String>,
    compute: F,
) -> Result<BatchCollectResult, CollectError>
where
    F: Fn(&str) -> Result<ComputedLeaf, CollectError> + Send + Sync + 'static,
{
    let outcomes = run_pipeline(inputs, output_dir, warnings, compute)?;
    let items: Vec<CollectItemResult> = outcomes.iter().map(shape_item).collect();
    Ok(BatchCollectResult {
        summary: summarize_collect_items(&items),
        items,
    })
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
