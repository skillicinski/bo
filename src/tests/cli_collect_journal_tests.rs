// Journal-stage tests: assert the engine journal records the right op/model
// for single-URL success, single-URL duplicate (no journal), and notes-only
// batch (no model). These drive the full pipeline via `collect_with_compute`
// and read back `engine::journal` events.

use crate::cli::collect::compute::ComputedLeaf;
use crate::cli::collect::{collect_with_compute, CollectError, CollectOutput};
use crate::domain::state;
use crate::domain::{Slug, Timestamp, Title, Url};
use std::fs;
use tempfile::TempDir;

#[test]
fn single_url_success_journals_one_collect_event_with_model() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let result = collect_with_compute(
        vec!["https://example.com/article".to_string()],
        dir.path(),
        "test-model",
        &mut Vec::new(),
        move |_| {
            Ok(ComputedLeaf {
                url: "https://example.com/article".to_string(),
                title: Some("Article".to_string()),
                body_markdown: "body".to_string(),
                summary_text: "summary".to_string(),
                note_warning: None,
            })
        },
    )
    .unwrap();
    assert!(matches!(result, CollectOutput::Single(_)));

    let events = crate::engine::journal::read_recent(dir.path(), 10);
    assert_eq!(events.len(), 1, "single success journals exactly one event");
    assert_eq!(events[0].op, crate::engine::journal::Op::Collect);
    assert_eq!(events[0].model.as_deref(), Some("test-model"));
}

#[test]
fn single_url_duplicate_errors_without_journaling() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let state_path = dir.path().join(".bo/state.json");
    crate::engine::state::write(
        &state_path,
        &state::TreeState {
            tree: state::TreeMetadata {
                name: "t".to_string(),
                created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves: vec![crate::domain::Leaf {
                slug: Slug::parse("article").unwrap(),
                file: "leaf/article.md".to_string(),
                title: Some(Title::parse("Article").unwrap()),
                url: Url::parse("https://example.com/article").unwrap(),
                collected_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                summary: None,
            }],
            branches: Vec::new(),
        },
    )
    .unwrap();

    // A single bare URL that is already collected must propagate the raw
    // DuplicateUrl error (exit 1) and, per the single-result contract, leave
    // no journal entry — not become a batch skip (which would exit 0 and
    // journal).
    let result = collect_with_compute(
        vec!["https://example.com/article".to_string()],
        dir.path(),
        "test-model",
        &mut Vec::new(),
        move |_| panic!("duplicate URL should not be computed"),
    );
    assert!(
        matches!(result, Err(CollectError::DuplicateUrl { .. })),
        "single duplicate must propagate DuplicateUrl, got {result:?}"
    );

    let events = crate::engine::journal::read_recent(dir.path(), 10);
    assert!(
        events.is_empty(),
        "single-URL failure must not journal: {events:?}"
    );
}

#[test]
fn notes_only_batch_journals_without_model() {
    let dir = TempDir::new().unwrap();
    let md = dir.path().join("note.md");
    fs::write(&md, "# A Note\n\nbody").unwrap();

    let result = collect_with_compute(
        vec![md.display().to_string()],
        dir.path(),
        "test-model",
        &mut Vec::new(),
        move |_| panic!("notes must not be fetched"),
    )
    .unwrap();
    assert!(matches!(result, CollectOutput::Batch(_)));

    let events = crate::engine::journal::read_recent(dir.path(), 10);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].op, crate::engine::journal::Op::Collect);
    // Notes collect no LLM summary, so the model is not recorded.
    assert!(events[0].model.is_none());
}

#[test]
fn failed_note_only_omits_model() {
    let dir = TempDir::new().unwrap();
    let md = dir.path().join("empty.md");
    fs::write(&md, "   \n  ").unwrap();

    let result = collect_with_compute(
        vec![md.display().to_string()],
        dir.path(),
        "test-model",
        &mut Vec::new(),
        move |_| panic!("notes must not be fetched"),
    )
    .unwrap();
    assert!(matches!(result, CollectOutput::Batch(_)));

    let events = crate::engine::journal::read_recent(dir.path(), 10);
    assert_eq!(events.len(), 1, "failed note journals exactly one event");
    assert_eq!(events[0].op, crate::engine::journal::Op::Collect);
    assert!(events[0].model.is_none());
}

#[test]
fn failed_web_batch_includes_model() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();

    let result = collect_with_compute(
        vec![
            "https://example.com/a".to_string(),
            "https://example.com/b".to_string(),
        ],
        dir.path(),
        "test-model",
        &mut Vec::new(),
        move |_| {
            Err(CollectError::Fetch(
                crate::engine::fetch::FetchError::Network("boom".to_string()),
            ))
        },
    )
    .unwrap();
    assert!(matches!(result, CollectOutput::Batch(_)));

    let events = crate::engine::journal::read_recent(dir.path(), 10);
    assert_eq!(
        events.len(),
        1,
        "failed web batch journals exactly one event"
    );
    assert_eq!(events[0].op, crate::engine::journal::Op::Collect);
    assert_eq!(events[0].model.as_deref(), Some("test-model"));
}

#[test]
fn mixed_note_web_batch_includes_model() {
    let dir = TempDir::new().unwrap();
    let md = dir.path().join("note.md");
    fs::write(&md, "# Note\n\nbody").unwrap();

    let result = collect_with_compute(
        vec![
            md.display().to_string(),
            "https://example.com/article".to_string(),
        ],
        dir.path(),
        "test-model",
        &mut Vec::new(),
        move |_| {
            Ok(ComputedLeaf {
                url: "https://example.com/article".to_string(),
                title: Some("Article".to_string()),
                body_markdown: "body".to_string(),
                summary_text: "summary".to_string(),
                note_warning: None,
            })
        },
    )
    .unwrap();
    assert!(matches!(result, CollectOutput::Batch(_)));

    let events = crate::engine::journal::read_recent(dir.path(), 10);
    assert_eq!(events.len(), 1, "mixed batch journals exactly one event");
    assert_eq!(events[0].op, crate::engine::journal::Op::Collect);
    assert_eq!(events[0].model.as_deref(), Some("test-model"));
}

#[test]
fn empty_url_list_omits_model() {
    // A URL-list file with no URLs expands to a single `empty_url_list`
    // Failure — no summary-eligible external source — so model is omitted
    // even though the batch journals a failed item.
    let dir = TempDir::new().unwrap();
    let list = dir.path().join("urls.txt");
    fs::write(&list, "\n   \n").unwrap();

    let result = collect_with_compute(
        vec![list.display().to_string()],
        dir.path(),
        "test-model",
        &mut Vec::new(),
        move |_| panic!("empty URL list must not be fetched"),
    )
    .unwrap();
    assert!(matches!(result, CollectOutput::Batch(_)));

    let events = crate::engine::journal::read_recent(dir.path(), 10);
    assert_eq!(events.len(), 1, "empty URL list journals exactly one event");
    assert_eq!(events[0].op, crate::engine::journal::Op::Collect);
    assert!(events[0].model.is_none());
}
