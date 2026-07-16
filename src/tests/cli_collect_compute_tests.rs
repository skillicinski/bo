// Compute-stage tests: fetch/extract/quality rejection and note leaf hashing.
// Assertions stay local to what `compute_leaf` / `compute_leaf_note` return.

use super::*;
use crate::cli::collect::{CollectError, NoteError};
use crate::domain::Url;
use std::fs;
use tempfile::TempDir;

#[test]
fn compute_leaf_url_rejects_blocked_http_status() {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    // Loopback HTTP fixture returning 403; classify_http_status maps 403 to
    // BlockedBySite. The rejection happens before summarize, so no LLM key
    // or external network is needed. stdlib-only — no mock HTTP crate.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = stream.flush();
        }
    });

    let url = format!("http://127.0.0.1:{port}/blocked");
    let err = compute_leaf(&url, &SummaryProvider::fallback()).unwrap_err();
    match err {
        CollectError::Rejected { url: u, reason } => {
            assert_eq!(u, url);
            assert_eq!(reason.to_string(), "blocked by site");
        }
        other => panic!("expected Rejected, got {other:?}"),
    }
}

#[test]
fn compute_leaf_note_strips_frontmatter_extracts_title_and_hashes_body() {
    let dir = TempDir::new().unwrap();
    let md = dir.path().join("note.md");
    fs::write(
        &md,
        "---\ntitle: My Note\ntags: [x]\n---\n# Heading One\n\nSome body text.\n",
    )
    .unwrap();

    let computed = compute_leaf_note(md.to_str().unwrap()).unwrap();
    assert_eq!(computed.title.as_deref(), Some("Heading One"));
    assert_eq!(computed.body_markdown, "Some body text.\n");
    assert!(computed.url.starts_with("bo://note/"));
    assert_eq!(computed.url.len(), "bo://note/".len() + 16);
    assert!(
        computed.note_warning.is_some(),
        "non-empty frontmatter should warn"
    );
    assert!(computed.summary_text.is_empty());
}

#[test]
fn compute_leaf_note_without_frontmatter_or_title_still_works() {
    let dir = TempDir::new().unwrap();
    let md = dir.path().join("plain.md");
    fs::write(&md, "just a body, no heading").unwrap();

    let computed = compute_leaf_note(md.to_str().unwrap()).unwrap();
    assert!(computed.title.is_none());
    assert!(computed.note_warning.is_none());
    // Untitled note slug derives from the synthetic url → note-<hash>.
    assert!(computed.url.starts_with("bo://note/"));
}

#[test]
fn compute_leaf_note_rejects_empty_after_frontmatter() {
    let dir = TempDir::new().unwrap();
    let md = dir.path().join("empty.md");
    fs::write(&md, "---\ntitle: x\n---\n\n").unwrap();

    let err = compute_leaf_note(md.to_str().unwrap()).unwrap_err();
    assert!(matches!(err, CollectError::Note(NoteError::Empty { .. })));
}

#[test]
fn compute_leaf_note_identical_content_yields_same_url() {
    let dir = TempDir::new().unwrap();
    let a = dir.path().join("a.md");
    let b = dir.path().join("b.md");
    fs::write(&a, "# Same\n\nbody").unwrap();
    fs::write(&b, "# Same\n\nbody").unwrap();

    let first = compute_leaf_note(a.to_str().unwrap()).unwrap();
    let second = compute_leaf_note(b.to_str().unwrap()).unwrap();
    assert_eq!(
        first.url, second.url,
        "same content must hash to one source url"
    );
}

#[test]
fn note_synthetic_url_parses() {
    // `bo` is a non-special scheme; url::Url must accept bo://note/<hex>.
    assert!(Url::parse("bo://note/0123456789abcdef").is_ok());
}
