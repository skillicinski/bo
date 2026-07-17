// bo journal — read the tree's operation journal.
//
// Read-only: never writes. Renders the append-only tier-3 log at
// `{tree}/.bo/journal.jsonl` as a paginated tail (newest last). A missing file
// is an empty journal, not an error — all commands work on trees that predate
// the journal.
//
// See internal/principles/derivation-tiers.md (tier 3) and issue #178.

use crate::engine::config::SeededConfig;
use crate::engine::journal::{self, Event};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

// ── public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct JournalResult {
    pub events: Vec<Event>,
}

pub fn run(cfg: &SeededConfig, limit: usize) -> JournalResult {
    let tree = cfg.tree();
    read(tree.path(), limit)
}

/// Read up to `limit` recent events (newest last). Missing file = empty.
pub fn read(tree_dir: &Path, limit: usize) -> JournalResult {
    JournalResult {
        events: journal::read_recent(tree_dir, limit),
    }
}

// ── human rendering ─────────────────────────────────────────────────────────

/// Compact one-line-per-event rendering. Newest last (matches `--json` order).
pub fn render_human(result: &JournalResult) -> String {
    if result.events.is_empty() {
        return String::from("no journal events yet\n");
    }
    let mut out = String::new();
    for event in &result.events {
        out.push_str(&render_event_line(event));
        out.push('\n');
    }
    out
}

fn render_event_line(event: &Event) -> String {
    let detail = match event.op {
        journal::Op::Collect => render_collect(&event.payload),
        journal::Op::Compile => render_compile(&event.payload),
        journal::Op::Repair => render_repair(&event.payload),
        journal::Op::Query => render_query(&event.payload),
    };
    let model = event
        .model
        .as_deref()
        .map(|m| format!("  model={m}"))
        .unwrap_or_default();
    format!(
        "{}  {:<7}  {}{}",
        event.ts,
        op_label(event.op),
        detail,
        model
    )
}

fn op_label(op: journal::Op) -> &'static str {
    match op {
        journal::Op::Collect => "collect",
        journal::Op::Compile => "compile",
        journal::Op::Repair => "repair",
        journal::Op::Query => "query",
    }
}

fn render_collect(payload: &Value) -> String {
    let items = payload.get("items").and_then(|v| v.as_array());
    let total = items.map_or(0, |a| a.len());
    let count = |status: &str| {
        items.map_or(0, |a| {
            a.iter()
                .filter(|i| i.get("status").and_then(|s| s.as_str()) == Some(status))
                .count()
        })
    };
    format!(
        "{} items ({} collected, {} skipped, {} failed)",
        total,
        count("collected"),
        count("skipped"),
        count("failed"),
    )
}

fn render_compile(payload: &Value) -> String {
    let mode = payload.get("mode").and_then(|v| v.as_str()).unwrap_or("?");
    let failures = payload
        .get("validation_failures")
        .and_then(|v| v.as_array());
    if let Some(failures) = failures {
        if !failures.is_empty() {
            let first = failures[0].as_str().unwrap_or("");
            let prefix: String = first.chars().take(80).collect();
            return format!("{mode}  validation failed: {prefix}");
        }
    }
    if let Some(error) = payload.get("error") {
        let code = error
            .get("code")
            .and_then(|v| v.as_str())
            .unwrap_or("error");
        let message = error.get("message").and_then(|v| v.as_str()).unwrap_or("");
        let prefix: String = message.chars().take(80).collect();
        return format!("{mode}  error: {code}: {prefix}");
    }
    let len = |key: &str| {
        payload
            .get(key)
            .and_then(|v| v.as_array())
            .map_or(0, |a| a.len())
    };
    let duration = payload
        .get("duration_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    format!(
        "{}  {} created, {} updated, {} deleted  {}ms",
        mode,
        len("branches_created"),
        len("branches_updated"),
        len("branches_deleted"),
        duration,
    )
}

fn render_repair(payload: &Value) -> String {
    let len = |key: &str| {
        payload
            .get(key)
            .and_then(|v| v.as_array())
            .map_or(0, |a| a.len())
    };
    format!(
        "{} orphan leaves pruned, {} branches repaired, {} branches removed",
        len("orphan_leaf_slugs"),
        len("repaired_branch_slugs"),
        len("removed_branches"),
    )
}

fn render_query(payload: &Value) -> String {
    let question = payload
        .get("question")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let citations = payload
        .get("citations")
        .and_then(|v| v.as_array())
        .map_or(0, |a| a.len());
    let consulted = payload
        .get("leaves_consulted")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    format!(
        "\"{}\"  {} citations  {} consulted",
        truncate(question, 60),
        citations,
        consulted
    )
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max).collect();
    format!("{truncated}\u{2026}")
}

#[cfg(test)]
#[path = "../tests/cli_journal_tests.rs"]
mod tests;
