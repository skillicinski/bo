// Tier-3 operation journal — append-only log of every operation a tree
// undergoes.
//
// One JSON event per line in `{tree}/.bo/journal.jsonl`. Lives outside the
// state file and outside the mutation transaction: appending is not a
// corpus mutation. Writes are best-effort — a journal failure must never fail
// or abort the user's command (warned to stderr at most). Readers tolerate a
// torn/partial final line. A missing file is an empty journal.
//
// See internal/principles/derivation-tiers.md (tier 3) and issue #178.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::domain::tree::infra_dir;
use crate::domain::Timestamp;

pub const SCHEMA_VERSION: u32 = 2;

/// Operation kind recorded in the journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Op {
    Collect,
    Synthesize,
    Repair,
    Query,
}

/// Who authored the event. `System` for command-driven operations; `User` for
/// editorial steering (not yet emitted — the envelope is shaped for it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    System,
    User,
}

/// A single journal event, serialized as one JSON line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub schema_version: u32,
    pub ts: String,
    pub op: Op,
    pub actor: Actor,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub payload: Value,
}

impl Event {
    /// Build a system-originated event stamped with the current time.
    pub fn system(op: Op, model: Option<String>, payload: Value) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            ts: Timestamp::now().to_rfc3339_millis(),
            op,
            actor: Actor::System,
            model,
            payload,
        }
    }
}

/// Path to the journal file for a tree.
pub fn journal_path(tree_dir: &Path) -> PathBuf {
    infra_dir(tree_dir).join("journal.jsonl")
}

/// Append an event as one line. Best-effort: errors are warned to stderr and
/// never propagated — a journal failure must not fail the command.
pub fn append(tree_dir: &Path, event: &Event) {
    let line = match serde_json::to_string(event) {
        Ok(line) => line,
        Err(error) => {
            tracing::warn!("journal: failed to encode {:?} event: {}", event.op, error);
            return;
        }
    };
    if let Err(error) = append_line(&journal_path(tree_dir), &line) {
        tracing::warn!("journal: failed to append {:?} event: {}", event.op, error);
    }
}

/// Append a system event with a typed payload. Convenience over [`append`]:
/// serializes `payload` to JSON and stamps the envelope.
pub fn append_payload<P: Serialize>(tree_dir: &Path, op: Op, model: Option<String>, payload: &P) {
    let payload = match serde_json::to_value(payload) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!("journal: failed to serialize {:?} payload: {}", op, error);
            return;
        }
    };
    append(tree_dir, &Event::system(op, model, payload));
}

/// Read up to `limit` most-recent events, oldest-to-newest (newest last).
/// Returns an empty vec when the journal does not exist or cannot be read;
/// unparseable lines (e.g. a torn final line) are skipped.
pub fn read_recent(tree_dir: &Path, limit: usize) -> Vec<Event> {
    let Ok(content) = std::fs::read_to_string(journal_path(tree_dir)) else {
        return Vec::new();
    };
    let mut events: Vec<Event> = content
        .lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| serde_json::from_str(line).ok())
        .filter(|event: &Event| event.schema_version == SCHEMA_VERSION)
        .collect();
    let start = events.len().saturating_sub(limit);
    events.drain(..start);
    events
}

fn append_line(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    // One write, not two: O_APPEND atomicity is per-write, and query appends
    // without the tree lock. Joining line+newline into a single write_all
    // closes the window where a concurrent append could land between them and
    // corrupt two events.
    file.write_all(format!("{line}\n").as_bytes())?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/engine_journal_tests.rs"]
mod tests;
