// Collect stage: journal payload and append. Best-effort — a journal failure
// never fails the command. `model` is recorded only when at least one
// summary-eligible external source (a real URL) was present in the expanded
// inputs, as determined at expand time and passed in explicitly.

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
/// failure never fails the command. `model` is included only when
/// `model_applicable` is true — that is, when at least one summary-eligible
/// external source (a real URL) was present in the expanded inputs. This is
/// derived from source classification at expand time and passed in explicitly,
/// so it is independent of credentials and per-item outcomes.
pub(super) fn journal(
    tree_dir: &Path,
    model: &str,
    model_applicable: bool,
    items: &[CollectItemResult],
) {
    let payload = CollectJournalPayload {
        items: items.iter().map(CollectJournalItem::from).collect(),
    };
    crate::engine::journal::append_payload(
        tree_dir,
        crate::engine::journal::Op::Collect,
        if model_applicable {
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
