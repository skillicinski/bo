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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_entry(url: &str, file: &str) -> IndexEntry {
        IndexEntry {
            file: file.to_string(),
            title: "Test Title".to_string(),
            url: url.to_string(),
        }
    }

    #[test]
    fn append_creates_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.jsonl");
        assert!(!path.exists());

        append_entry(&path, &make_entry("https://example.com", "example.md")).unwrap();
        assert!(path.exists());

        let entries = read_index(&path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].url, "https://example.com");
    }

    #[test]
    fn append_multiple_valid_jsonl() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.jsonl");

        append_entry(&path, &make_entry("https://a.com", "a.md")).unwrap();
        append_entry(&path, &make_entry("https://b.com", "b.md")).unwrap();
        append_entry(&path, &make_entry("https://c.com", "c.md")).unwrap();

        let entries = read_index(&path).unwrap();
        assert_eq!(entries.len(), 3);

        // Verify each line is independently valid JSON with exactly the right fields
        let content = fs::read_to_string(&path).unwrap();
        for line in content.lines() {
            let parsed: serde_json::Value = serde_json::from_str(line)
                .unwrap_or_else(|_| panic!("line is not valid JSON: {line}"));
            assert!(parsed.get("file").is_some());
            assert!(parsed.get("title").is_some());
            assert!(parsed.get("url").is_some());
            assert_eq!(
                parsed.as_object().unwrap().len(),
                3,
                "index entry must have exactly 3 fields"
            );
        }
    }

    #[test]
    fn duplicate_detection_exact_match() {
        let entries = vec![make_entry("https://example.com/article", "article.md")];
        assert!(is_duplicate(&entries, "https://example.com/article").is_some());
    }

    #[test]
    fn near_duplicate_not_detected() {
        let entries = vec![make_entry("https://example.com/article", "article.md")];
        assert!(is_duplicate(&entries, "https://example.com/article?ref=twitter").is_none());
    }

    #[test]
    fn read_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.jsonl");
        fs::write(&path, "").unwrap();

        let entries = read_index(&path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn read_nonexistent_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.jsonl");
        let entries = read_index(&path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn skips_malformed_lines() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.jsonl");
        let content = r#"{"file":"good.md","title":"Good","url":"https://good.com"}
this is not json
{"file":"also-good.md","title":"Also Good","url":"https://also-good.com"}
"#;
        fs::write(&path, content).unwrap();

        let entries = read_index(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].url, "https://good.com");
        assert_eq!(entries[1].url, "https://also-good.com");
    }

    #[test]
    fn failed_url_not_in_index_can_resubmit() {
        // Only successful collects go in the index.
        // A URL that failed previously should not be blocked.
        let entries = vec![make_entry("https://example.com/success", "success.md")];
        assert!(is_duplicate(&entries, "https://example.com/failed").is_none());
    }

    #[test]
    fn empty_title_stored_as_empty_string() {
        let entry = IndexEntry {
            file: "no-title.md".to_string(),
            title: String::new(),
            url: "https://example.com/no-title".to_string(),
        };
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("index.jsonl");
        append_entry(&path, &entry).unwrap();

        let entries = read_index(&path).unwrap();
        assert_eq!(entries[0].title, "");
    }
}
