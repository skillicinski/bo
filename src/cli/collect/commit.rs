// Collect stage: the single-atomic-commit boundary.
//
// Dedup against the on-disk state, allocate disambiguated slugs, stage
// writes through the pending transaction, and shape pipeline outcomes into
// the public `CollectItemResult`/`CollectResult` records.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::domain::slug::Slug;
use crate::domain::state::{self, TreeMetadata, TreeState};
use crate::domain::{leaf, slug, Leaf, Timestamp, Title, Url};
use crate::engine::pending::{self, OpKind, PendingWrite};

use super::compute::{compute_leaf_note, ComputedLeaf};
use super::error_code;
use super::input::ExpandedCollectInput;
use super::{
    BatchCollectSummary, CollectError, CollectItemResult, CollectItemStatus, CollectResult,
};

pub(super) fn duplicate_file(url: &str, output_dir: &Path) -> Result<Option<String>, CollectError> {
    let state_path = output_dir.join(".bo").join("state.json");
    let state = match crate::engine::state::read(&state_path) {
        Ok(m) => m,
        Err(state::TreeStateError::TreeNotInitialized) => return Ok(None),
        Err(e) => return Err(CollectError::TreeState(e)),
    };
    Ok(state
        .leaves
        .iter()
        .find(|l| l.url.as_str() == url)
        .map(|l| l.file.clone()))
}

// ── phase 1: dedup (sequential) ─────────────────────────────────────────────

/// Plan produced by [`dedup_inputs`]: shaped outcomes for failures/duplicates,
/// the URL set still needing compute, and notes already computed inline.
pub(super) struct DedupPlan {
    pub(super) outcomes: Vec<Outcome>,
    pub(super) to_compute: Vec<(String, String)>,
    pub(super) precomputed_notes: Vec<(String, ComputedLeaf)>,
}

/// Expand-classified inputs into dedup outcomes + a compute work list. Notes
/// are computed inline (no fetch) and carried pre-computed; URL-list read/empty
/// failures are shaped here (input owns classification, commit owns shaping).
pub(super) fn dedup_inputs(expanded: Vec<ExpandedCollectInput>, output_dir: &Path) -> DedupPlan {
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut outcomes: Vec<Outcome> = Vec::new();
    let mut to_compute: Vec<(String, String)> = Vec::new();
    // Notes are computed inline (no fetch) and carried to phase 3 pre-computed.
    let mut precomputed_notes: Vec<(String, ComputedLeaf)> = Vec::new();

    for input in expanded {
        let (input_label, url) = match input {
            ExpandedCollectInput::Url { input, url, .. } => (input, url),
            ExpandedCollectInput::Failure {
                input,
                code,
                message,
            } => {
                outcomes.push(Outcome::Item(collect_failure_item(
                    &input, None, &code, message,
                )));
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

    DedupPlan {
        outcomes,
        to_compute,
        precomputed_notes,
    }
}

/// Recovery notices are stderr-bound diagnostics: the pipeline collects them,
/// the CLI renders post-run (#138 presentation-purity contract).
pub(super) fn recover_pending_if_needed(
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
pub(super) fn existing_slug_stems(output_dir: &Path) -> HashSet<String> {
    let mut stems = HashSet::new();
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

/// Load the state from disk, or return an empty one if the tree is freshly seeded.
pub(super) fn load_or_bootstrap_state(
    output_dir: &Path,
    now: &Timestamp,
) -> Result<TreeState, CollectError> {
    let state_path = output_dir.join(".bo").join("state.json");
    match crate::engine::state::read(&state_path) {
        Ok(m) => Ok(m),
        Err(state::TreeStateError::TreeNotInitialized) => Ok(TreeState {
            tree: TreeMetadata {
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
        Err(e) => Err(CollectError::TreeState(e)),
    }
}

/// Atomically commit a state update with staged writes and deletes.
///
/// Writes the pending operation, stages content, writes the state, then
/// applies writes/deletes and clears the pending file. The entire sequence
/// is guarded by the pending lock so a crash mid-commit is recoverable.
pub(super) fn commit_state_and_writes(
    output_dir: &Path,
    op: OpKind,
    state: &TreeState,
    staged: &[(&PendingWrite, &[u8])],
    deletes: &[String],
) -> Result<(), CollectError> {
    pending::commit_with_state(output_dir, op, state, staged, deletes).map_err(
        |error| match error {
            pending::PendingError::TreeState(error) => CollectError::TreeState(error),
            other => CollectError::Io(std::io::Error::other(other.to_string())),
        },
    )
}

// ── phase 3: sequential commit ──────────────────────────────────────────────

/// Commit computed leaves in input order: allocate disambiguated slugs, stage
/// leaf writes through the pending transaction, and single-atomically commit.
/// Failures and same-batch duplicates append to `outcomes`; note warnings land
/// in `warnings`. TreeState/slug/pending logic is unchanged from the inlined loop.
pub(super) fn commit_computed(
    compute_results: Vec<(String, String, Result<ComputedLeaf, CollectError>)>,
    output_dir: &Path,
    outcomes: &mut Vec<Outcome>,
    warnings: &mut Vec<String>,
) -> Result<(), CollectError> {
    let now = Timestamp::now();
    let mut state = load_or_bootstrap_state(output_dir, &now)?;
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
                if let Some(existing) = state.leaves.iter().find(|l| l.url.as_str() == computed.url)
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
                let filename = slug::allocate_slug(&base_slug, &computed.url, &mut used_slugs);
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

                state.leaves.push(Leaf {
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
        commit_state_and_writes(
            output_dir,
            OpKind::Collect {
                url: format!("batch of {} urls", staged_refs.len()),
            },
            &state,
            &staged_refs,
            &[],
        )?;
    }

    Ok(())
}

// ── item constructors ────────────────────────────────────────────────────────

pub(super) fn collect_success_item(input: &str, result: CollectResult) -> CollectItemResult {
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

pub(super) fn collect_skipped_item(
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

pub(super) fn collect_failure_item(
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

pub(super) fn collect_item_from_error(
    input: &str,
    url: &str,
    error: &CollectError,
) -> CollectItemResult {
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

pub(super) fn summarize_collect_items(items: &[CollectItemResult]) -> BatchCollectSummary {
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

// ── outcome ──────────────────────────────────────────────────────────────────

/// Internal result from the pipeline, before shaping into output variants.
pub(super) enum Outcome {
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

// ── shapers ──────────────────────────────────────────────────────────────────

pub(super) fn shape_item(o: &Outcome) -> CollectItemResult {
    match o {
        Outcome::Collected { input, result } => collect_success_item(input, result.clone()),
        Outcome::Errored { input, url, error } => collect_item_from_error(input, url, error),
        Outcome::Item(item) => item.clone(),
    }
}

pub(super) fn shape_single(outcomes: Vec<Outcome>) -> Result<CollectResult, CollectError> {
    match outcomes.into_iter().next() {
        Some(Outcome::Collected { result, .. }) => Ok(result),
        Some(Outcome::Errored { error, .. }) => Err(error),
        Some(Outcome::Item(_)) => unreachable!("single bare URL cannot produce a synthetic item"),
        None => unreachable!("single input produces one outcome"),
    }
}

#[cfg(test)]
#[path = "../../tests/cli_collect_commit_tests.rs"]
mod tests;
