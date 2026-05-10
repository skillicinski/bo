// JSONL index — derived cache of document frontmatter.
//
// The index is a thin navigation lookup: given a URL, find the filename;
// given a list of documents, show titles. Timestamps and other per-document
// metadata live exclusively in each document's frontmatter, which is the
// authoritative record.
//
// NOTE: duplicate detection reads the index only (fast path). If index.jsonl
// is absent or manually deleted, bo add treats every URL as new and will
// silently produce a duplicate. A frontmatter-scan fallback is deferred to
// a future bo compile rebuild command.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexEntry {
    pub file: String,
    pub title: String,
    pub url: String,
}

/// Read all index entries from a JSONL file.
/// Returns an empty vec if the file doesn't exist.
/// Skips malformed lines with a warning to stderr.
pub fn read_index(path: &Path) -> io::Result<Vec<IndexEntry>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut entries = Vec::new();
    for (i, line) in reader.lines().enumerate() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<IndexEntry>(trimmed) {
            Ok(entry) => entries.push(entry),
            Err(e) => {
                eprintln!("warning: skipping malformed index line {}: {}", i + 1, e);
            }
        }
    }
    Ok(entries)
}

/// Check if a URL already exists in the index (exact string match).
pub fn is_duplicate<'a>(entries: &'a [IndexEntry], url: &str) -> Option<&'a IndexEntry> {
    entries.iter().find(|e| e.url == url)
}

/// Append a single entry to the index file.
/// Creates the file if it doesn't exist.
pub fn append_entry(path: &Path, entry: &IndexEntry) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    let json = serde_json::to_string(entry).map_err(io::Error::other)?;
    writeln!(file, "{}", json)?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/domain_index_tests.rs"]
mod tests;
