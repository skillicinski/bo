use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn raw_json_mode_detection_stops_at_arg_terminator() {
    assert!(raw_json_mode_requested(&[
        OsString::from("bo"),
        OsString::from("search"),
        OsString::from("--json"),
    ]));
    assert!(!raw_json_mode_requested(&[
        OsString::from("bo"),
        OsString::from("search"),
        OsString::from("--"),
        OsString::from("--json"),
    ]));
}

#[test]
fn compile_validation_json_error_includes_next_action() {
    let error = compile_json_error(&CompileError::Validation(
        "invalid compile response".to_string(),
    ));

    assert_eq!(error.code, "validation_error");
    assert_eq!(error.message, "invalid compile response");
    assert_eq!(error.details["phase"], "compile_validation");
    assert_eq!(error.details["files_changed"], false);
    assert_eq!(error.details["next_step"], compile::VALIDATION_NEXT_STEP);
}

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
        ExpandedCollectInput::Url {
            input,
            url,
            from_file,
        } => {
            assert!(input.ends_with("urls.txt:1"), "input was {input}");
            assert_eq!(url, "https://example.com/one");
            assert!(*from_file);
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
        ExpandedCollectInput::Failure { item, from_file } => {
            assert!(*from_file);
            assert_eq!(item.status, CollectItemStatus::Failed);
            assert_eq!(item.code.as_deref(), Some("url_list_read_error"));
        }
        other => panic!("unexpected expanded input: {other:?}"),
    }
}

#[test]
fn batch_collect_deduplicates_repeated_input_urls() {
    let dir = TempDir::new().unwrap();
    let mut calls = 0;
    let url = "https://example.com/article".to_string();

    let result = execute_collect_with_collector(
        vec![url.clone(), url.clone()],
        dir.path(),
        |collected_url| {
            calls += 1;
            Ok(collect::Document {
                url: collected_url.to_string(),
                filename: format!("article-{calls}.md"),
            })
        },
    )
    .unwrap();

    let CollectOutput::Batch(result) = result else {
        panic!("expected batch result");
    };
    assert_eq!(calls, 1);
    assert_eq!(result.summary.collected, 1);
    assert_eq!(result.summary.skipped, 1);
    assert_eq!(result.summary.failed, 0);
    assert_eq!(result.items[1].status, CollectItemStatus::Skipped);
    assert_eq!(result.items[1].code.as_deref(), Some("duplicate_input"));
}

#[test]
fn batch_collect_skips_existing_index_duplicates_without_fetching() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    let url = "https://example.com/already";
    bo::domain::index::append_entry(
        &dir.path().join(".bo/index.jsonl"),
        &bo::domain::index::IndexEntry {
            file: "already.md".to_string(),
            title: "Already".to_string(),
            url: url.to_string(),
        },
    )
    .unwrap();

    let result = execute_collect_with_collector(
        vec![url.to_string(), url.to_string()],
        dir.path(),
        |_url| panic!("duplicate URL should not be fetched"),
    )
    .unwrap();

    let CollectOutput::Batch(result) = result else {
        panic!("expected batch result");
    };
    assert_eq!(result.summary.collected, 0);
    assert_eq!(result.summary.skipped, 2);
    assert_eq!(result.summary.failed, 0);
    assert_eq!(result.items[0].code.as_deref(), Some("duplicate_url"));
    assert_eq!(result.items[0].existing_file.as_deref(), Some("already.md"));
    assert_eq!(result.items[1].code.as_deref(), Some("duplicate_input"));
}
