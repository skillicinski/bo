use super::*;
use crate::engine::journal::{Actor, Event, Op, SCHEMA_VERSION};
use serde_json::json;

fn event(op: Op, model: Option<&str>, payload: serde_json::Value) -> Event {
    Event {
        schema_version: SCHEMA_VERSION,
        ts: "2026-07-13T10:00:00.000Z".to_string(),
        op,
        actor: Actor::System,
        model: model.map(String::from),
        payload,
    }
}

#[test]
fn empty_renders_hint() {
    let out = render_human(&JournalResult { events: vec![] });
    assert_eq!(out, "no journal events yet\n");
}

#[test]
fn collect_renders_item_counts() {
    let e = event(
        Op::Collect,
        Some("gpt-4.1-mini"),
        json!({
            "items": [
                {"input":"https://a","status":"collected","url":"https://a","file":"leaf/a.md"},
                {"input":"https://b","status":"skipped","url":"https://b"},
                {"input":"https://c","status":"failed","url":"https://c"},
            ]
        }),
    );
    let out = render_human(&JournalResult { events: vec![e] });
    assert!(out.contains("collect"));
    assert!(out.contains("3 items (1 collected, 1 skipped, 1 failed)"));
    assert!(out.contains("model=gpt-4.1-mini"));
}

#[test]
fn compile_success_renders_branch_counts() {
    let e = event(
        Op::Compile,
        Some("gpt-4.1"),
        json!({
            "mode": "full",
            "new_leaf_slugs": ["a", "b"],
            "branches_created": [{"slug": "x", "title": "X", "leaf_count": 2}],
            "branches_updated": [],
            "branches_deleted": ["old"],
            "validation_failures": [],
            "duration_ms": 3200
        }),
    );
    let out = render_human(&JournalResult { events: vec![e] });
    assert!(out.contains("full"));
    assert!(out.contains("1 created, 0 updated, 1 deleted"));
    assert!(out.contains("3200ms"));
}

#[test]
fn compile_failure_renders_validation_message() {
    let e = event(
        Op::Compile,
        Some("gpt-4.1"),
        json!({
            "mode": "incremental",
            "new_leaf_slugs": [],
            "branches_created": [],
            "branches_updated": [],
            "branches_deleted": [],
            "validation_failures": ["branch #1 has empty title"],
            "duration_ms": 120
        }),
    );
    let out = render_human(&JournalResult { events: vec![e] });
    assert!(out.contains("validation failed: branch #1 has empty title"));
}

#[test]
fn compile_error_renders_code_and_message() {
    let e = event(
        Op::Compile,
        Some("gpt-4.1"),
        json!({
            "mode": "full",
            "new_leaf_slugs": [],
            "branches_created": [],
            "branches_updated": [],
            "branches_deleted": [],
            "validation_failures": [],
            "error": {"code": "truncated", "message": "compile output was truncated"},
            "duration_ms": 5000
        }),
    );
    let out = render_human(&JournalResult { events: vec![e] });
    assert!(out.contains("full  error: truncated: compile output was truncated"));
}

#[test]
fn repair_renders_prune_counts_and_omits_model() {
    let e = event(
        Op::Repair,
        None,
        json!({
            "orphan_leaf_slugs": ["a"],
            "repaired_branch_slugs": ["b", "c"],
            "removed_branches": [
                {"slug": "d", "title": "D", "remaining_leaf_count": 1, "reason": "stale_branch_below_minimum_leaves"}
            ]
        }),
    );
    let out = render_human(&JournalResult { events: vec![e] });
    assert!(out.contains("repair"));
    assert!(out.contains("1 orphan leaves pruned, 2 branches repaired, 1 branches removed"));
    assert!(
        !out.contains("model="),
        "repair is deterministic; model must be absent"
    );
}

#[test]
fn query_renders_question_and_citations() {
    let e = event(
        Op::Query,
        Some("gpt-4.1-mini"),
        json!({
            "question": "what is ownership in rust",
            "answer": "...",
            "citations": [{"slug": "rust-ownership", "title": "Rust Ownership", "file": "leaf/rust-ownership.md"}],
            "leaves_consulted": 3
        }),
    );
    let out = render_human(&JournalResult { events: vec![e] });
    assert!(out.contains("query"));
    assert!(out.contains("\"what is ownership in rust\""));
    assert!(out.contains("1 citations"));
    assert!(out.contains("3 consulted"));
}

#[test]
fn json_result_serializes_events_array() {
    let e = event(Op::Collect, None, json!({}));
    let result = JournalResult { events: vec![e] };
    let s = serde_json::to_string(&result).unwrap();
    assert!(s.contains("\"events\""));
    assert!(s.contains("\"op\":\"collect\""));
}
