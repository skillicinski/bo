// Input-stage tests: classify/expand URLs, URL-list files, and local notes.

use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn collect_input_expands_txt_url_list() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("urls.txt");
    fs::write(
        &path,
        " https://example.com/one \n\nhttps://example.com/two\n",
    )
    .unwrap();

    let expanded = expand_collect_inputs(&[path.display().to_string()]);

    assert_eq!(expanded.len(), 2);
    match &expanded[0] {
        ExpandedCollectInput::Url { input, url, .. } => {
            assert!(input.ends_with("urls.txt:1"), "input was {input}");
            assert_eq!(url, "https://example.com/one");
        }
        other => panic!("unexpected expanded input: {other:?}"),
    }
    match &expanded[1] {
        ExpandedCollectInput::Url { input, url, .. } => {
            assert!(input.ends_with("urls.txt:3"), "input was {input}");
            assert_eq!(url, "https://example.com/two");
        }
        other => panic!("unexpected expanded input: {other:?}"),
    }
}

#[test]
fn collect_input_treats_missing_local_txt_as_url_list_error() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("missing.txt");

    let expanded = expand_collect_inputs(&[path.display().to_string()]);

    assert_eq!(expanded.len(), 1);
    match &expanded[0] {
        ExpandedCollectInput::Failure { code, .. } => {
            assert_eq!(code, "url_list_read_error");
        }
        other => panic!("unexpected expanded input: {other:?}"),
    }
}

#[test]
fn is_local_note_file_requires_existing_md_without_scheme() {
    let dir = TempDir::new().unwrap();
    let md = dir.path().join("note.md");
    fs::write(&md, "body").unwrap();

    assert!(is_local_note_file(md.to_str().unwrap()));
    assert!(!is_local_note_file(
        dir.path().join("missing.md").to_str().unwrap()
    ));
    assert!(!is_local_note_file("https://example.com/page.md"));
    let txt = dir.path().join("list.txt");
    fs::write(&txt, "https://example.com").unwrap();
    assert!(!is_local_note_file(txt.to_str().unwrap()));
}

#[test]
fn expand_routes_existing_md_to_note() {
    let dir = TempDir::new().unwrap();
    let md = dir.path().join("n.md");
    fs::write(&md, "body").unwrap();

    let expanded = expand_collect_inputs(&[md.display().to_string()]);
    assert!(matches!(expanded[0], ExpandedCollectInput::Note { .. }));
}

#[test]
fn is_single_bare_url_distinguishes_inputs() {
    // A lone bare URL selects the single-result contract.
    assert!(is_single_bare_url(&["https://example.com".to_string()]));
    // A URL whose path ends in .txt is still a single URL (has a scheme).
    assert!(is_single_bare_url(&[
        "https://example.com/feed.txt".to_string()
    ]));
    // A bare .txt argument routes to batch (mirrors pre-unification routing).
    assert!(!is_single_bare_url(&["urls.txt".to_string()]));
    // A nested mixed-case .TXT list routes to batch: is_url_list_file (used by
    // expansion) recognises .txt case-insensitively, so the output-policy
    // check must agree, or shape_single would report only the first outcome.
    assert!(!is_single_bare_url(&["data/urls.TXT".to_string()]));
    assert!(!is_single_bare_url(&["data/URLS.txt".to_string()]));
    // Multiple inputs always route to batch.
    assert!(!is_single_bare_url(&[
        "https://a.com".to_string(),
        "https://b.com".to_string()
    ]));
}
