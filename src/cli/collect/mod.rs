// bo collect — the collect pipeline.
//
// Single URLs, URL lists, multi-URL, and local notes all route through ONE
// pipeline: expand → dedup → compute → single-atomic-commit. The summary
// provider is resolved once per invocation and shared across worker threads.
// `CollectOutput::Single` vs `Batch` is output policy only — a single bare
// URL returns the single-result contract; every other input shape returns
// a batch.
//
// Stage layout (this module holds the command API types + orchestrator):
//   input   — classify/expand URLs, URL-list files, local notes.
//   compute — acquire/normalize web/YouTube/note content into a `ComputedLeaf`.
//   commit  — dedup, slug allocation, state mutation, pending transaction,
//             item constructors, and `Outcome`/output shaping.
//   journal — collect journal payload (engine journal append).
//   render  — human-readable output.
//
// Dependency direction: collect → adapters, fetch, quality, extract, leaf, slug, state, pending.

use serde::Serialize;
use serde_json::json;
use std::fmt;
use std::path::Path;
use std::sync::Arc;
use std::thread;

use crate::adapters::youtube::YoutubeError;
use crate::cli::json::JsonError;
use crate::domain::state;
use crate::engine::pending;
use crate::engine::quality::RejectReason;
use crate::engine::{extract, fetch};

mod commit;
mod compute;
mod input;
mod journal;
mod render;

pub(crate) use input::is_single_bare_url;
pub use render::{render_batch_human, render_human};

use commit::{
    commit_computed, dedup_inputs, recover_pending_if_needed, shape_item, shape_single,
    summarize_collect_items, DedupPlan, Outcome,
};
use compute::{compute_leaf, ComputedLeaf, SummaryProvider};
use input::expand_collect_inputs;

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
    TreeState(state::TreeStateError),
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
            CollectError::TreeState(e) => write!(f, "{}", e),
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

impl From<state::TreeStateError> for CollectError {
    fn from(e: state::TreeStateError) -> Self {
        CollectError::TreeState(e)
    }
}

impl From<pending::PendingError> for CollectError {
    fn from(e: pending::PendingError) -> Self {
        CollectError::Pending(e)
    }
}

pub(super) fn error_code(error: &CollectError) -> &'static str {
    match error {
        CollectError::DuplicateUrl { .. } => "duplicate_url",
        CollectError::Rejected { .. } => "rejected",
        CollectError::Fetch(_) => "fetch_error",
        CollectError::Extract(_) => "extract_error",
        CollectError::Youtube(_) => "youtube_error",
        CollectError::Io(_) => "io_error",
        CollectError::TreeState(_) | CollectError::Pending(pending::PendingError::TreeState(_)) => {
            "state_error"
        }
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

// ── unified pipeline ─────────────────────────────────────────────────────────

/// Expand, dedup, compute, and single-atomic-commit. Returns outcomes so the
/// caller can shape them into `CollectOutput::Single` or `CollectOutput::Batch`,
/// plus a flag indicating whether any expanded input is a summary-eligible
/// external source (drives journal model applicability).
fn run_pipeline<F>(
    inputs: Vec<String>,
    output_dir: &Path,
    warnings: &mut Vec<String>,
    compute: F,
) -> Result<(Vec<Outcome>, bool), CollectError>
where
    F: Fn(&str) -> Result<ComputedLeaf, CollectError> + Send + Sync + 'static,
{
    recover_pending_if_needed(output_dir, warnings)?;
    let expanded = expand_collect_inputs(&inputs);
    let model_applicable = input::has_external_source(&expanded);

    let DedupPlan {
        mut outcomes,
        to_compute,
        precomputed_notes,
    } = dedup_inputs(expanded, output_dir);

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

    commit_computed(compute_results, output_dir, &mut outcomes, warnings)?;

    Ok((outcomes, model_applicable))
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
    if is_single_bare_url(&inputs) {
        eprintln!("fetching {}...", inputs[0]);
    }
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
    let (outcomes, model_applicable) = run_pipeline(inputs, output_dir, warnings, compute)?;
    let items: Vec<CollectItemResult> = outcomes.iter().map(shape_item).collect();
    if single {
        // A single bare URL propagates its raw error (duplicate/rejected/fetch)
        // and, per the single-result contract, is not journaled on failure.
        match shape_single(outcomes) {
            Ok(result) => {
                journal::journal(output_dir, model, model_applicable, &items);
                Ok(CollectOutput::Single(result))
            }
            Err(error) => Err(error),
        }
    } else {
        journal::journal(output_dir, model, model_applicable, &items);
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
    let (outcomes, _) = run_pipeline(inputs, output_dir, warnings, compute)?;
    let items: Vec<CollectItemResult> = outcomes.iter().map(shape_item).collect();
    Ok(BatchCollectResult {
        summary: summarize_collect_items(&items),
        items,
    })
}

#[cfg(test)]
#[path = "../../tests/cli_collect_tests.rs"]
mod tests;
