// Write-intent recovery for manifest-backed tree mutations.
//
// `{tree}/.bo/pending.json` is both an intent log and an advisory lock. A
// mutating command writes it before staging content, commits by rewriting or
// deleting `manifest.json`, then renames staged files / applies deletes and
// clears pending. On the next mutating command, a stale pending file is rolled
// back or rolled forward according to whether the manifest hash changed.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

const LIVE_LOCK_WINDOW_SECS: i64 = 60;
const MISSING_MANIFEST_HASH: &str = "<missing>";

// ── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingOperation {
    pub op: OpKind,
    pub started_at: String,
    pub pid: u32,
    pub pre_manifest_hash: String,
    pub writes: Vec<PendingWrite>,
    #[serde(default)]
    pub deletes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum OpKind {
    Collect { url: String },
    Compile { mode: CompileMode },
    Raze { include_auth: bool },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum CompileMode {
    Incremental,
    Full,
    RebuildStale,
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
pub enum PendingError {
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

impl fmt::Display for PendingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PendingError::Io(e) => write!(f, "pending I/O error: {e}"),
            PendingError::Parse(e) => write!(f, "pending parse error: {e}"),
            PendingError::Busy { tree_dir } => write!(
                f,
                "another bo process is already interacting with {}",
                tree_dir.display()
            ),
            PendingError::SuspiciousPath { path } => {
                write!(f, "pending operation contains suspicious path: {path}")
            }
            PendingError::HashMismatch {
                path,
                expected,
                actual,
            } => write!(
                f,
                "staged write hash mismatch for {path}: expected {expected}, got {actual}"
            ),
            PendingError::MissingStagedWrite { path } => {
                write!(f, "missing staged write for pending path: {path}")
            }
        }
    }
}

impl From<io::Error> for PendingError {
    fn from(e: io::Error) -> Self {
        PendingError::Io(e)
    }
}

impl From<serde_json::Error> for PendingError {
    fn from(e: serde_json::Error) -> Self {
        PendingError::Parse(e)
    }
}

// ── core API ─────────────────────────────────────────────────────────────────

pub fn pending_path(tree_dir: &Path) -> PathBuf {
    tree_dir.join(".bo").join("pending.json")
}

pub fn read(path: &Path) -> Result<Option<PendingOperation>, PendingError> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents)
            .map(Some)
            .map_err(PendingError::Parse),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(PendingError::Io(e)),
    }
}

pub fn write(path: &Path, pending: &PendingOperation) -> Result<(), PendingError> {
    let json = serde_json::to_string_pretty(pending)?;
    atomic_write(path, json.as_bytes())?;
    Ok(())
}

pub fn clear(path: &Path) -> Result<(), PendingError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PendingError::Io(e)),
    }
}

pub fn new_operation(
    tree_dir: &Path,
    op: OpKind,
    writes: Vec<PendingWrite>,
    deletes: Vec<String>,
) -> Result<PendingOperation, PendingError> {
    Ok(PendingOperation {
        op,
        started_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        pid: std::process::id(),
        pre_manifest_hash: manifest_hash(tree_dir)?,
        writes,
        deletes,
    })
}

/// Recover a stale pending operation or refuse if it belongs to a live process.
///
/// Returns `Ok(Some(report))` only when a stale pending file existed and was
/// resolved. Callers should surface the report as a one-line warning.
pub fn recover_or_refuse(tree_dir: &Path) -> Result<Option<RecoveryReport>, PendingError> {
    let path = pending_path(tree_dir);
    let Some(pending) = read(&path)? else {
        return Ok(None);
    };

    if is_live_lock(&pending) {
        return Err(PendingError::Busy {
            tree_dir: tree_dir.to_path_buf(),
        });
    }

    let current_hash = manifest_hash(tree_dir)?;
    let changes = if current_hash == pending.pre_manifest_hash {
        rollback(tree_dir, &pending)?
    } else {
        roll_forward(tree_dir, &pending)?
    };
    clear(&path)?;

    Ok(Some(RecoveryReport {
        op: op_label(&pending.op).to_string(),
        changes,
    }))
}

pub fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

pub fn manifest_hash(tree_dir: &Path) -> Result<String, PendingError> {
    hash_file_or_missing(&tree_dir.join(".bo").join("manifest.json"))
}

pub fn write_staged(
    tree_dir: &Path,
    write: &PendingWrite,
    bytes: &[u8],
) -> Result<(), PendingError> {
    if content_hash(bytes) != write.content_hash {
        return Err(PendingError::HashMismatch {
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

pub fn apply_writes(tree_dir: &Path, writes: &[PendingWrite]) -> Result<usize, PendingError> {
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
            return Err(PendingError::HashMismatch {
                path: write.path.clone(),
                expected: write.content_hash.clone(),
                actual,
            });
        }

        return Err(PendingError::MissingStagedWrite {
            path: write.path.clone(),
        });
    }
    Ok(changes)
}

pub fn apply_deletes(tree_dir: &Path, deletes: &[String]) -> Result<usize, PendingError> {
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
            Err(e) => return Err(PendingError::Io(e)),
        }
    }
    Ok(changes)
}

/// Atomic commit: write pending, stage content, write manifest, apply writes
/// and deletes, clear pending. Used by collect and compile — they share
/// the same 6-step transaction idiom.
pub fn commit_with_manifest(
    tree_dir: &Path,
    op: OpKind,
    manifest: &crate::domain::manifest::Manifest,
    staged: &[(&PendingWrite, &[u8])],
    deletes: &[String],
) -> Result<(), PendingError> {
    let writes: Vec<PendingWrite> = staged.iter().map(|(pw, _)| (*pw).clone()).collect();
    let operation = new_operation(tree_dir, op, writes.clone(), deletes.to_vec())?;
    let pending_path = pending_path(tree_dir);
    write(&pending_path, &operation)?;
    for (pw, bytes) in staged {
        write_staged(tree_dir, pw, bytes)?;
    }
    let manifest_path = tree_dir.join(".bo").join("manifest.json");
    crate::domain::manifest::write(&manifest_path, manifest)
        .map_err(|e| PendingError::Io(std::io::Error::other(e.to_string())))?;
    apply_writes(tree_dir, &writes)?;
    apply_deletes(tree_dir, deletes)?;
    clear(&pending_path)?;
    Ok(())
}

pub fn staged_path(tree_dir: &Path, relative: &str) -> Result<PathBuf, PendingError> {
    let final_path = resolve_relative(tree_dir, relative)?;
    let mut staged = final_path.as_os_str().to_owned();
    staged.push(".tmp");
    Ok(PathBuf::from(staged))
}

// ── recovery internals ───────────────────────────────────────────────────────

fn rollback(tree_dir: &Path, pending: &PendingOperation) -> Result<usize, PendingError> {
    let mut changes = 0usize;
    for write in &pending.writes {
        let staged = staged_path(tree_dir, &write.path)?;
        match fs::remove_file(&staged) {
            Ok(()) => changes += 1,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(PendingError::Io(e)),
        }
    }
    Ok(changes)
}

fn roll_forward(tree_dir: &Path, pending: &PendingOperation) -> Result<usize, PendingError> {
    let writes = apply_writes(tree_dir, &pending.writes)?;
    let deletes = apply_deletes(tree_dir, &pending.deletes)?;
    Ok(writes + deletes)
}

fn verify_file_hash(path: &Path, write: &PendingWrite) -> Result<(), PendingError> {
    let actual = hash_file_or_missing(path)?;
    if actual == write.content_hash {
        return Ok(());
    }
    Err(PendingError::HashMismatch {
        path: write.path.clone(),
        expected: write.content_hash.clone(),
        actual,
    })
}

fn is_live_lock(pending: &PendingOperation) -> bool {
    is_process_alive(pending.pid) && is_recent(&pending.started_at)
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

fn op_label(op: &OpKind) -> &'static str {
    match op {
        OpKind::Collect { .. } => "collect",
        OpKind::Compile { .. } => "compile",
        OpKind::Raze { .. } => "raze",
    }
}

fn resolve_relative(tree_dir: &Path, relative: &str) -> Result<PathBuf, PendingError> {
    let path = Path::new(relative);
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(PendingError::SuspiciousPath {
            path: relative.to_string(),
        });
    }
    Ok(tree_dir.join(path))
}

fn hash_file_or_missing(path: &Path) -> Result<String, PendingError> {
    match fs::read(path) {
        Ok(bytes) => Ok(content_hash(&bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(MISSING_MANIFEST_HASH.to_string()),
        Err(e) => Err(PendingError::Io(e)),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut tmp_path = path.as_os_str().to_owned();
    tmp_path.push(".tmp");
    let tmp_path = PathBuf::from(tmp_path);
    {
        let mut f = fs::File::create(&tmp_path)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp_path, path)?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/engine_pending_tests.rs"]
mod tests;
