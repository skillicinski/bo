use super::*;
// unused domain imports removed
use std::fs;
use tempfile::TempDir;

fn pending(op: OpKind, pre_state_hash: String, writes: Vec<PendingWrite>) -> PendingOperation {
    PendingOperation {
        op,
        started_at: "2026-05-19T00:00:00Z".to_string(),
        pid: 0,
        pre_state_hash,
        writes,
        deletes: Vec::new(),
    }
}

#[test]
fn write_read_clear_round_trip() {
    let dir = TempDir::new().unwrap();
    let path = pending_path(dir.path());
    let op = pending(
        OpKind::Collect {
            url: "https://example.com".to_string(),
        },
        "abc".to_string(),
        vec![PendingWrite {
            path: "leaf.md".to_string(),
            content_hash: content_hash(b"body"),
        }],
    );

    write(&path, &op).unwrap();
    assert_eq!(read(&path).unwrap(), Some(op));
    clear(&path).unwrap();
    assert!(read(&path).unwrap().is_none());
}

#[test]
fn rollback_removes_staged_write_when_state_unchanged() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    fs::write(dir.path().join(".bo/state.json"), b"state").unwrap();
    let pre = state_hash(dir.path()).unwrap();
    let body = b"new leaf";
    let write = PendingWrite {
        path: "leaf.md".to_string(),
        content_hash: content_hash(body),
    };
    let op = pending(
        OpKind::Collect {
            url: "https://example.com".to_string(),
        },
        pre,
        vec![write.clone()],
    );
    super::write(&pending_path(dir.path()), &op).unwrap();
    write_staged(dir.path(), &write, body).unwrap();

    let report = recover_or_refuse(dir.path()).unwrap().unwrap();

    assert_eq!(report.op, "collect");
    assert_eq!(report.changes, 1);
    assert!(!staged_path(dir.path(), "leaf.md").unwrap().exists());
    assert!(!dir.path().join("leaf.md").exists());
    assert!(read(&pending_path(dir.path())).unwrap().is_none());
}

#[test]
fn roll_forward_renames_staged_write_when_state_changed() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    fs::write(dir.path().join(".bo/state.json"), b"state-before").unwrap();
    let pre = state_hash(dir.path()).unwrap();
    let body = b"new leaf";
    let write = PendingWrite {
        path: "leaf.md".to_string(),
        content_hash: content_hash(body),
    };
    let op = pending(
        OpKind::Collect {
            url: "https://example.com".to_string(),
        },
        pre,
        vec![write.clone()],
    );
    super::write(&pending_path(dir.path()), &op).unwrap();
    write_staged(dir.path(), &write, body).unwrap();
    fs::write(dir.path().join(".bo/state.json"), b"state-after").unwrap();

    let report = recover_or_refuse(dir.path()).unwrap().unwrap();

    assert_eq!(report.changes, 1);
    assert_eq!(fs::read(dir.path().join("leaf.md")).unwrap(), body);
    assert!(!staged_path(dir.path(), "leaf.md").unwrap().exists());
    assert!(read(&pending_path(dir.path())).unwrap().is_none());
}

#[test]
fn live_pending_refuses() {
    let dir = TempDir::new().unwrap();
    fs::create_dir_all(dir.path().join(".bo")).unwrap();
    fs::write(dir.path().join(".bo/state.json"), b"state").unwrap();
    let op = PendingOperation {
        op: OpKind::Compile {
            mode: CompileMode::Full,
        },
        started_at: Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        pid: std::process::id(),
        pre_state_hash: state_hash(dir.path()).unwrap(),
        writes: Vec::new(),
        deletes: Vec::new(),
    };
    write(&pending_path(dir.path()), &op).unwrap();

    let err = recover_or_refuse(dir.path()).unwrap_err();
    assert!(matches!(err, PendingError::Busy { .. }));
}
