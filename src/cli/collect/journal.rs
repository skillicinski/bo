// Collect stage: journal payload and append. Best-effort — a journal failure
// never fails the command. `model` is recorded only when at least one real
// (non-note) URL was processed.

use std::path::Path;

use serde::Serialize;

use super::{CollectItemResult, CollectItemStatus};

#[derive(Serialize)]
pub(super) struct CollectJournalItem<'a> {
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
pub(super) struct CollectJournalPayload<'a> {
    items: Vec<CollectJournalItem<'a>>,
}

/// Record a collect operation in the tree's journal. Best-effort: a journal
/// failure never fails the command. `model` is included only when at least one
/// real (non-note) URL was processed — notes collect no LLM summary.
pub(super) fn journal(tree_dir: &Path, model: &str, items: &[CollectItemResult]) {
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

#[cfg(test)]
#[path = "../../tests/cli_collect_journal_tests.rs"]
mod tests;
