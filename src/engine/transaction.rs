// Write-intent recovery for state-backed tree mutations.
//
// `{tree}/.bo/pending.json` is both an intent log and an advisory lock. A
// mutating command writes it before staging content, commits by rewriting or
// deleting `state.json`, then renames staged files / applies deletes and
// clears the pending transaction. On the next mutating command, a stale pending file is rolled
// back or rolled forward according to whether the state hash changed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const LIVE_LOCK_WINDOW_SECS: i64 = 60;
const MISSING_STATE_HASH: &str = "<missing>";

// ── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingTransaction {
    pub op: TransactionKind,
    pub started_at: String,
    pub pid: u32,
    pub pre_state_hash: String,
    pub writes: Vec<PendingWrite>,
    #[serde(default)]
    pub deletes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum TransactionKind {
    Collect { url: String },
    Synthesize { mode: SynthesisMode },
    Raze { include_auth: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum SynthesisMode {
    Incremental,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingWrite {
    /// Final path relative to tree root. The staging path is `{path}.tmp`.
    pub path: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryReport {
    pub op: String,
    pub changes: usize,
}

#[derive(Debug)]
pub enum TransactionError {
    Io(io::Error),
    Parse(serde_json::Error),
    Busy {
        tree_dir: PathBuf,
    },
    SuspiciousPath {
        path: String,
    },
    HashMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    MissingStagedWrite {
        path: String,
    },
}

impl fmt::Display for TransactionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransactionError::Io(e) => write!(f, "transaction I/O error: {e}"),
            TransactionError::Parse(e) => write!(f, "transaction parse error: {e}"),
            TransactionError::Busy { tree_dir } => write!(
                f,
                "another bo process is already interacting with {}",
                tree_dir.display()
            ),
            TransactionError::SuspiciousPath { path } => {
                write!(f, "transaction contains suspicious path: {path}")
            }
            TransactionError::HashMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "staged write hash mismatch for {path}: expected {expected}, got {actual}"
            ),
            TransactionError::MissingStagedWrite { path } => {
                write!(f, "missing staged write for transaction path: {path}")
            }
        }
    }
}

impl From<io::Error> for TransactionError {
    fn from(e: io::Error) -> Self {
        TransactionError::Io(e)
    }
}

impl From<serde_json::Error> for TransactionError {
    fn from(e: serde_json::Error) -> Self {
        TransactionError::Parse(e)
    }
}

// ── core API ─────────────────────────────────────────────────────────────────

pub fn pending_path(tree_dir: &Path) -> PathBuf {
    tree_dir.join(".bo").join("pending.json")
}

pub fn read(path: &Path) -> Result<Option<PendingTransaction>, TransactionError> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map(Some)
            .map_err(TransactionError::Parse),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(TransactionError::Io(e)),
    }
}

pub fn write(path: &Path, transaction: &PendingTransaction) -> Result<(), TransactionError> {
    let json = serde_json::to_string_pretty(transaction)?;
    crate::engine::state::atomic_write(path, json.as_bytes())?;
    Ok(())
}

pub fn clear(path: &Path) -> Result<(), TransactionError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(TransactionError::Io(e)),
    }
}

pub fn new_transaction(
    tree_dir: &Path,
    op: TransactionKind,
    writes: Vec<PendingWrite>,
    deletes: Vec<String>,
) -> Result<PendingTransaction, TransactionError> {
    Ok(PendingTransaction {
        op,
        started_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        pid: std::process::id(),
        pre_state_hash: state_hash(tree_dir)?,
        writes,
        deletes,
    })
}

/// Recover a stale pending transaction or refuse if it belongs to a live process.
///
/// Returns `Ok(Some(report))` only when a stale pending file existed and was
/// resolved. Callers should surface the report as a one-line warning.
pub fn recover_or_refuse(tree_dir: &Path) -> Result<Option<RecoveryReport>, TransactionError> {
    let path = pending_path(tree_dir);
    let Some(transaction) = read(&path)? else {
        return Ok(None);
    };

    if is_live_lock(&transaction) {
        return Err(TransactionError::Busy {
            tree_dir: tree_dir.to_path_buf(),
        });
    }

    let current_hash = state_hash(tree_dir)?;
    let changes = if current_hash == transaction.pre_state_hash {
        rollback(tree_dir, &transaction)?
    } else {
        roll_forward(tree_dir, &transaction)?
    };
    clear(&path)?;

    Ok(Some(RecoveryReport {
        op: kind_label(&transaction.op).to_string(),
        changes,
    }))
}

pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn state_hash(tree_dir: &Path) -> Result<String, TransactionError> {
    hash_file_or_missing(&crate::domain::tree::state_path(tree_dir))
}

pub fn write_staged(
    tree_dir: &Path,
    write: &PendingWrite,
    bytes: &[u8],
) -> Result<(), TransactionError> {
    if content_hash(bytes) != write.content_hash {
        return Err(TransactionError::HashMismatch {
            path: write.path.clone(),
            expected: write.content_hash.clone(),
            actual: content_hash(bytes),
        });
    }

    let staged = staged_path(tree_dir, &write.path)?;
    if let Some(parent) = staged.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = fs::File::create(&staged)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

pub fn apply_writes(tree_dir: &Path, writes: &[PendingWrite]) -> Result<usize, TransactionError> {
    let mut changes = 0usize;
    for write in writes {
        let staged = staged_path(tree_dir, &write.path)?;
        let final_path = resolve_relative(tree_dir, &write.path)?;

        if staged.exists() {
            verify_file_hash(&staged, write)?;
            if let Some(parent) = final_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::rename(&staged, &final_path)?;
            changes += 1;
            continue;
        }

        if final_path.exists() {
            let actual = hash_file_or_missing(&final_path)?;
            if actual == write.content_hash {
                continue;
            }
            return Err(TransactionError::HashMismatch {
                path: write.path.clone(),
                expected: write.content_hash.clone(),
                actual,
            });
        }

        return Err(TransactionError::MissingStagedWrite {
            path: write.path.clone(),
        });
    }
    Ok(changes)
}

pub fn apply_deletes(tree_dir: &Path, deletes: &[String]) -> Result<usize, TransactionError> {
    let mut changes = 0usize;
    for delete in deletes {
        let path = resolve_relative(tree_dir, delete)?;
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
            changes += 1;
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => changes += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(TransactionError::Io(e)),
        }
    }
    Ok(changes)
}

/// Atomic commit: write the pending transaction, stage content, write tree state,
/// apply writes and deletes, then clear the transaction. Used by collect and synthesize — they share
/// the same 6-step transaction idiom.
pub fn commit_with_state(
    tree_dir: &Path,
    op: TransactionKind,
    state: &crate::domain::state::TreeState,
    staged: &[(&PendingWrite, &[u8])],
    deletes: &[String],
) -> Result<(), TransactionError> {
    let writes: Vec<PendingWrite> = staged.iter().map(|(pw, _)| (*pw).clone()).collect();
    let transaction = new_transaction(tree_dir, op, writes.clone(), deletes.to_vec())?;
    let pending_path = pending_path(tree_dir);
    write(&pending_path, &transaction)?;
    for (pw, bytes) in staged {
        write_staged(tree_dir, pw, bytes)?;
    }
    let state_path = crate::domain::tree::state_path(tree_dir);
    crate::engine::state::write(&state_path, state).map_err(
        |error: crate::domain::state::TreeStateError| {
            TransactionError::Io(std::io::Error::other(error.to_string()))
        },
    )?;
    apply_writes(tree_dir, &writes)?;
    apply_deletes(tree_dir, deletes)?;
    clear(&pending_path)?;
    Ok(())
}

pub fn staged_path(tree_dir: &Path, relative: &str) -> Result<PathBuf, TransactionError> {
    let final_path = resolve_relative(tree_dir, relative)?;
    let mut staged = final_path.as_os_str().to_owned();
    staged.push(".tmp");
    Ok(PathBuf::from(staged))
}

// ── recovery internals ───────────────────────────────────────────────────────

fn rollback(tree_dir: &Path, transaction: &PendingTransaction) -> Result<usize, TransactionError> {
    let mut changes = 0usize;
    for write in &transaction.writes {
        let staged = staged_path(tree_dir, &write.path)?;
        match fs::remove_file(&staged) {
            Ok(()) => changes += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(TransactionError::Io(e)),
        }
    }
    Ok(changes)
}

fn roll_forward(
    tree_dir: &Path,
    transaction: &PendingTransaction,
) -> Result<usize, TransactionError> {
    let writes = apply_writes(tree_dir, &transaction.writes)?;
    let deletes = apply_deletes(tree_dir, &transaction.deletes)?;
    Ok(writes + deletes)
}

fn verify_file_hash(path: &Path, write: &PendingWrite) -> Result<(), TransactionError> {
    let actual = hash_file_or_missing(path)?;
    if actual == write.content_hash {
        return Ok(());
    }
    Err(TransactionError::HashMismatch {
        path: write.path.clone(),
        expected: write.content_hash.clone(),
        actual,
    })
}

fn is_live_lock(transaction: &PendingTransaction) -> bool {
    is_process_alive(transaction.pid) && is_recent(&transaction.started_at)
}

fn is_recent(started_at: &str) -> bool {
    let Ok(parsed) = DateTime::parse_from_rfc3339(started_at) else {
        return false;
    };
    let age = Utc::now().signed_duration_since(parsed.with_timezone(&Utc));
    age.num_seconds() < LIVE_LOCK_WINDOW_SECS
}

fn is_process_alive(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }

    Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn kind_label(kind: &TransactionKind) -> &'static str {
    match kind {
        TransactionKind::Collect { .. } => "collect",
        TransactionKind::Synthesize { .. } => "synthesize",
        TransactionKind::Raze { .. } => "raze",
    }
}

fn resolve_relative(tree_dir: &Path, relative: &str) -> Result<PathBuf, TransactionError> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(TransactionError::SuspiciousPath {
            path: relative.to_string(),
        });
    }
    Ok(tree_dir.join(path))
}

fn hash_file_or_missing(path: &Path) -> Result<String, TransactionError> {
    match fs::read(path) {
        Ok(bytes) => Ok(content_hash(&bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(MISSING_STATE_HASH.to_string()),
        Err(e) => Err(TransactionError::Io(e)),
    }
}

#[cfg(test)]
#[path = "../tests/engine_transaction_tests.rs"]
mod tests;
