use super::*;
use std::io::Write;
use tempfile::TempDir;

fn tree_dir() -> TempDir {
    TempDir::new().unwrap()
}

#[test]
fn missing_journal_reads_as_empty() {
    let dir = tree_dir();
    assert!(read_recent(dir.path(), 20).is_empty());
}

#[test]
fn append_then_read_roundtrip_preserves_order() {
    let dir = tree_dir();
    append(
        dir.path(),
        &Event::system(
            Op::Collect,
            Some("gpt-4.1-mini".into()),
            serde_json::json!({"items":[]}),
        ),
    );
    append(
        dir.path(),
        &Event::system(
            Op::Query,
            Some("gpt-4.1-mini".into()),
            serde_json::json!({"question":"q"}),
        ),
    );
    let events = read_recent(dir.path(), 20);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].op, Op::Collect);
    assert_eq!(events[1].op, Op::Query);
    assert_eq!(events[0].schema_version, SCHEMA_VERSION);
    assert_eq!(events[0].actor, Actor::System);
    assert!(!events[0].ts.is_empty(), "ts must be populated");
}

#[test]
fn read_returns_tail_up_to_limit_newest_last() {
    let dir = tree_dir();
    for i in 0..5u32 {
        append(
            dir.path(),
            &Event::system(Op::Collect, None, serde_json::json!({"i": i})),
        );
    }
    let events = read_recent(dir.path(), 2);
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload["i"], serde_json::json!(3));
    assert_eq!(events[1].payload["i"], serde_json::json!(4));
}

#[test]
fn torn_final_line_is_tolerated() {
    let dir = tree_dir();
    append(
        dir.path(),
        &Event::system(Op::Collect, None, serde_json::json!({})),
    );
    append(
        dir.path(),
        &Event::system(Op::Query, None, serde_json::json!({})),
    );
    // Append a partial (torn) line with no trailing newline.
    let path = journal_path(dir.path());
    let mut file = OpenOptions::new().append(true).open(&path).unwrap();
    file.write_all(b"{\"op\":\"repair\",\"paylo").unwrap();
    let events = read_recent(dir.path(), 20);
    assert_eq!(events.len(), 2, "torn final line should be skipped");
}

#[test]
fn append_creates_missing_infra_dir() {
    let dir = tree_dir();
    assert!(!dir.path().join(".bo").exists());
    append(
        dir.path(),
        &Event::system(Op::Collect, None, serde_json::json!({})),
    );
    assert!(journal_path(dir.path()).exists());
}

#[test]
fn append_payload_serializes_typed_struct() {
    let dir = tree_dir();
    #[derive(serde::Serialize)]
    struct Payload {
        mode: &'static str,
        n: u32,
    }
    append_payload(
        dir.path(),
        Op::Compile,
        Some("gpt-4.1".into()),
        &Payload { mode: "full", n: 3 },
    );
    let events = read_recent(dir.path(), 20);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].op, Op::Compile);
    assert_eq!(events[0].model.as_deref(), Some("gpt-4.1"));
    assert_eq!(events[0].payload["mode"], "full");
    assert_eq!(events[0].payload["n"], serde_json::json!(3));
}

#[test]
fn model_is_omitted_when_none() {
    let dir = tree_dir();
    append(
        dir.path(),
        &Event::system(Op::Repair, None, serde_json::json!({})),
    );
    let raw = std::fs::read_to_string(journal_path(dir.path())).unwrap();
    assert!(!raw.contains("model"), "raw line was: {raw}");
}
