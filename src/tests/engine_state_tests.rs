use super::{read_state, slug_from_filename, write_state, TreeState};
use std::collections::HashMap;
use tempfile::TempDir;

#[test]
fn read_write_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.json");

    let mut state = TreeState::default();
    state
        .compiled_leaves
        .insert("foo".to_string(), "abc123".to_string());
    state
        .compiled_leaves
        .insert("bar".to_string(), "def456".to_string());

    write_state(&path, &state).unwrap();
    let loaded = read_state(&path);

    assert_eq!(loaded.compiled_leaves, state.compiled_leaves);
}

#[test]
fn read_absent_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("nonexistent.json");

    let state = read_state(&path);
    assert!(state.compiled_leaves.is_empty());
}

#[test]
fn read_malformed_file() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("state.json");
    std::fs::write(&path, b"this is not json at all {{{{").unwrap();

    let state = read_state(&path);
    assert!(state.compiled_leaves.is_empty());
}

#[test]
fn write_creates_parent_dirs() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("a").join("b").join("c").join("state.json");

    let state = TreeState {
        compiled_leaves: HashMap::new(),
    };

    write_state(&path, &state).unwrap();
    assert!(path.exists());
}

#[test]
fn slug_from_filename_strips_md() {
    assert_eq!(slug_from_filename("foo-bar.md"), "foo-bar");
}

#[test]
fn slug_from_filename_no_extension() {
    assert_eq!(slug_from_filename("foo-bar"), "foo-bar");
}
