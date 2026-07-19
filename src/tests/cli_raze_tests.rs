use super::*;
use crate::domain::Timestamp;
use std::fs;
use std::io::Cursor;
use std::path::PathBuf;
use tempfile::TempDir;

// Inlined from the former src/tests/fixtures/mod.rs (single-consumer).
// Each helper constructs bo's typed records and stages them under a temp tree.

fn title(s: &str) -> crate::domain::Title {
    crate::domain::Title::parse(s).expect("invalid test title")
}

fn url(s: &str) -> crate::domain::Url {
    crate::domain::Url::parse(s).expect("invalid test url")
}

/// Create a minimal seeded tree with an empty state inside `tmp`.
/// Returns `(tree_dir, config_path)`.
fn setup_tree(tmp: &TempDir) -> (PathBuf, PathBuf) {
    let tree_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");
    fs::create_dir_all(&tree_dir).unwrap();
    let bo_dir = tree_dir.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    crate::engine::state::write(
        &bo_dir.join("state.json"),
        &crate::domain::state::TreeState {
            tree: crate::domain::state::TreeMetadata {
                name: "tree".to_string(),
                created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        },
    )
    .unwrap();

    crate::engine::config::write_config(
        &crate::engine::config::Config {
            provider: crate::engine::llm::Provider::OpenAI,
            tree: Some(crate::domain::tree::TreeConfig {
                path: tree_dir.clone(),
                name: "tree".to_string(),
                created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            }),
            model: "gpt-4.1-mini".to_string(),
            compile_model: None,
            base_url: None,
        },
        &config_path,
    )
    .unwrap();

    (tree_dir, config_path)
}

fn auth_path_for_config(config_path: &Path) -> PathBuf {
    config_path.with_file_name("auth.json")
}

/// Add a leaf file (`.md`) and a corresponding state entry.
fn add_leaf(tree_dir: &Path, file: &str) {
    add_state_leaf(tree_dir, file);
    let path = tree_dir.join(file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, "# content\n").unwrap();
}

/// Add a state entry for a leaf without creating the file on disk.
fn add_state_leaf(tree_dir: &Path, file: &str) {
    static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let state_path = tree_dir.join(".bo/state.json");
    let mut state = crate::engine::state::read(&state_path).unwrap();
    let idx = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let slug = crate::domain::Slug::parse(&format!("leaf-{}", idx)).unwrap();
    state.leaves.push(crate::domain::Leaf {
        slug,
        file: file.to_string(),
        title: Some(title(file.trim_end_matches(".md"))),
        url: url("https://example.com/test"),
        collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
        summary: None,
    });
    crate::engine::state::write(&state_path, &state).unwrap();
}

/// Construct a `Leaf` with sensible defaults for tests.
fn make_leaf_record(slug: &str, file: &str) -> crate::domain::Leaf {
    crate::domain::Leaf {
        slug: crate::domain::Slug::parse(slug).expect("invalid test slug"),
        file: file.to_string(),
        title: Some(title(file.trim_end_matches(".md"))),
        url: url(&format!("https://example.com/{slug}")),
        collected_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
        summary: None,
    }
}

/// Construct a `TreeState` with the given leaves and empty branches.
fn make_state(name: &str, leaves: Vec<crate::domain::Leaf>) -> crate::domain::state::TreeState {
    crate::domain::state::TreeState {
        tree: crate::domain::state::TreeMetadata {
            name: name.to_string(),
            created_at: Timestamp::parse("2025-01-01T00:00:00Z").unwrap(),
            last_compiled_at: None,
        },
        leaves,
        branches: Vec::new(),
    }
}

#[test]
fn deletes_tracked_files() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    add_leaf(&tree_dir, "a.md");
    add_leaf(&tree_dir, "b.md");

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.result.deleted_files, 2);
    assert!(!tree_dir.join("a.md").exists());
    assert!(!tree_dir.join("b.md").exists());
}

#[test]
fn deletes_state_file() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(output.result.deleted_state);
    assert!(!tree_dir.join(".bo").exists());
}

#[test]
fn deletes_state_alongside_other_infra() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    // Pre-T6.1, raze removes the entire .bo/ directory which incidentally
    // wipes the state. This test pins the behaviour and guards against
    // anyone reintroducing a state path that escapes infra teardown.
    let state_path = tree_dir.join(".bo/state.json");
    crate::engine::state::write(
        &state_path,
        &crate::domain::state::TreeState {
            tree: crate::domain::state::TreeMetadata {
                name: "raze-test".to_string(),
                created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                last_compiled_at: None,
            },
            leaves: Vec::new(),
            branches: Vec::new(),
        },
    )
    .unwrap();
    assert!(state_path.exists());

    let _ = raze(&tree_dir, &config_path).unwrap();

    assert!(!state_path.exists(), "state.json must be deleted by raze");
}

#[test]
fn removes_empty_output_directory() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(output.result.removed_output_dir);
    assert!(!tree_dir.exists());
}

#[test]
fn leaves_non_empty_directory_in_place() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    fs::write(tree_dir.join("stray.txt"), "not tracked").unwrap();

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(!output.result.removed_output_dir);
    assert!(output.result.output_dir_left_in_place);
    assert!(tree_dir.exists());
}

#[test]
fn deletes_config_file() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    assert!(config_path.exists());

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(output.result.deleted_config);
    assert!(!config_path.exists());
}

#[test]
fn preserves_auth_file_by_default() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    let auth_path = auth_path_for_config(&config_path);
    fs::write(
        &auth_path,
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(!output.result.deleted_auth);
    assert!(output.result.preserved_auth);
    assert_eq!(output.result.auth_path, auth_path.display().to_string());
    assert!(auth_path.exists());
}

#[test]
fn include_auth_deletes_auth_file() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    let auth_path = auth_path_for_config(&config_path);
    fs::write(
        &auth_path,
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();

    let output = raze_with_auth(&tree_dir, &config_path, &auth_path, AuthCleanup::Delete).unwrap();

    assert!(output.result.deleted_auth);
    assert!(!output.result.preserved_auth);
    assert_eq!(output.result.auth_path, auth_path.display().to_string());
    assert!(!auth_path.exists());
}

#[test]
fn missing_auth_file_is_tolerated() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);

    let output = raze(&tree_dir, &config_path).unwrap();

    assert!(!output.result.deleted_auth);
    assert!(!output.result.preserved_auth);
}

#[test]
fn auth_only_cleanup_deletes_auth_without_tree_config() {
    let tmp = TempDir::new().unwrap();
    let auth_path = tmp.path().join("auth.json");
    fs::write(
        &auth_path,
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();

    let output = raze_auth_only(&auth_path).unwrap().unwrap();

    assert!(output.result.deleted_auth);
    assert!(!output.result.deleted_config);
    assert!(!auth_path.exists());
    assert!(render_human(&output.result).contains("deleted auth"));
}

#[test]
fn skips_missing_files_without_error() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    add_state_leaf(&tree_dir, "ghost.md");

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.result.deleted_files, 0);
    assert!(output.warnings.is_empty());
}

#[test]
fn warns_on_suspicious_path_traversal() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    add_state_leaf(&tree_dir, "../escape.md");

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.warnings.len(), 1);
    assert_eq!(output.warnings[0].code, "suspicious_state_entry");
    assert_eq!(output.result.deleted_files, 0);
}

#[test]
fn warns_on_absolute_path_in_state_entry() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);
    add_state_leaf(&tree_dir, "/etc/passwd");

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.warnings.len(), 1);
    assert_eq!(output.warnings[0].code, "suspicious_state_entry");
}

#[test]
fn empty_tree_produces_zero_deletes() {
    let tmp = TempDir::new().unwrap();
    let (tree_dir, config_path) = setup_tree(&tmp);

    let output = raze(&tree_dir, &config_path).unwrap();

    assert_eq!(output.result.deleted_files, 0);
    assert!(output.result.deleted_state);
}

#[test]
fn render_human_includes_file_count() {
    let result = RazeResult {
        cancelled: false,
        deleted_files: 3,
        deleted_state: true,
        removed_output_dir: true,
        output_dir_left_in_place: false,
        deleted_config: true,
        deleted_auth: true,
        preserved_auth: false,
        output_dir: "/tmp/tree".to_string(),
        config_path: "/tmp/.bo/config.json".to_string(),
        auth_path: "/tmp/.bo/auth.json".to_string(),
    };
    let output = render_human(&result);
    assert!(output.contains("3 markdown file(s)"));
    assert!(output.contains("deleted state"));
    assert!(output.contains("removed output directory"));
    assert!(output.contains("deleted config"));
    assert!(output.contains("deleted auth"));
}

#[test]
fn render_human_shows_dir_left_in_place() {
    let result = RazeResult {
        cancelled: false,
        deleted_files: 0,
        deleted_state: false,
        removed_output_dir: false,
        output_dir_left_in_place: true,
        deleted_config: false,
        deleted_auth: false,
        preserved_auth: false,
        output_dir: "/tmp/tree".to_string(),
        config_path: "/tmp/.bo/config.json".to_string(),
        auth_path: "/tmp/.bo/auth.json".to_string(),
    };
    let output = render_human(&result);
    assert!(output.contains("left in place"));
}

#[test]
fn render_human_shows_preserved_auth() {
    let result = RazeResult {
        cancelled: false,
        deleted_files: 0,
        deleted_state: false,
        removed_output_dir: false,
        output_dir_left_in_place: false,
        deleted_config: true,
        deleted_auth: false,
        preserved_auth: true,
        output_dir: "/tmp/tree".to_string(),
        config_path: "/tmp/.bo/config.json".to_string(),
        auth_path: "/tmp/.bo/auth.json".to_string(),
    };
    let output = render_human(&result);
    assert!(output.contains("preserved auth"));
    assert!(output.contains("/tmp/.bo/auth.json"));
}

// ─── confirmation gate tests ────────────────────────────────────────────────

#[test]
fn confirm_accepts_exact_yes() {
    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    let result = confirm_raze(
        std::path::Path::new("/tmp/tree"),
        None,
        false,
        &mut reader,
        &mut writer,
    )
    .unwrap();
    assert!(result);
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("Type 'yes' to confirm:"));
}

#[test]
fn confirm_rejects_variants() {
    for input in &["Yes\n", "YES\n", "y\n", "\n"] {
        let mut reader = Cursor::new(input.as_bytes());
        let mut writer = Vec::new();
        let result = confirm_raze(
            std::path::Path::new("/tmp/tree"),
            None,
            false,
            &mut reader,
            &mut writer,
        )
        .unwrap();
        assert!(!result, "should reject {:?}", input);
    }
}

#[test]
fn confirm_rejects_random() {
    for input in &["no\n", "maybe\n", "foo\n", "yes \n"] {
        let mut reader = Cursor::new(input.as_bytes());
        let mut writer = Vec::new();
        let result = confirm_raze(
            std::path::Path::new("/tmp/tree"),
            None,
            false,
            &mut reader,
            &mut writer,
        )
        .unwrap();
        assert!(!result, "should reject {:?}", input);
    }
}

#[test]
fn confirm_shows_tree_path() {
    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    confirm_raze(
        std::path::Path::new("/home/user/my-tree"),
        None,
        false,
        &mut reader,
        &mut writer,
    )
    .unwrap();
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("/home/user/my-tree"));
}

#[test]
fn confirm_shows_leaf_and_branch_counts() {
    let state = make_state(
        "test",
        vec![
            make_leaf_record("leaf-1", "a.md"),
            make_leaf_record("leaf-2", "b.md"),
        ],
    );

    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    confirm_raze(
        std::path::Path::new("/tmp/tree"),
        Some(&state),
        false,
        &mut reader,
        &mut writer,
    )
    .unwrap();
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("2 leaves, 0 branches"));
}

#[test]
fn confirm_missing_state_shows_degraded_message() {
    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    confirm_raze(
        std::path::Path::new("/tmp/tree"),
        None,
        false,
        &mut reader,
        &mut writer,
    )
    .unwrap();
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("unable to read state"));
}

#[test]
fn confirm_with_auth_shows_will_be_deleted() {
    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    confirm_raze(
        std::path::Path::new("/tmp/tree"),
        None,
        true,
        &mut reader,
        &mut writer,
    )
    .unwrap();
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("will be deleted"));
}

#[test]
fn confirm_auth_only_accepts_yes() {
    let mut reader = Cursor::new(b"yes\n");
    let mut writer = Vec::new();
    let result = confirm_auth_only(
        std::path::Path::new("/tmp/.bo/auth.json"),
        &mut reader,
        &mut writer,
    )
    .unwrap();
    assert!(result);
    let output = String::from_utf8(writer).unwrap();
    assert!(output.contains("Type 'yes' to confirm:"));
    assert!(output.contains("/tmp/.bo/auth.json"));
}

#[test]
fn confirm_auth_only_rejects_non_yes() {
    for input in &["Yes\n", "YES\n", "y\n", "\n", "no\n", "yes \n"] {
        let mut reader = Cursor::new(input.as_bytes());
        let mut writer = Vec::new();
        let result = confirm_auth_only(
            std::path::Path::new("/tmp/.bo/auth.json"),
            &mut reader,
            &mut writer,
        )
        .unwrap();
        assert!(!result, "should reject {:?}", input);
    }
}

#[test]
fn render_human_cancelled_shows_message() {
    let result = RazeResult {
        cancelled: true,
        deleted_files: 0,
        deleted_state: false,
        removed_output_dir: false,
        output_dir_left_in_place: false,
        deleted_config: false,
        deleted_auth: false,
        preserved_auth: false,
        output_dir: String::new(),
        config_path: String::new(),
        auth_path: String::new(),
    };
    let output = render_human(&result);
    assert_eq!(output, "raze cancelled\n");
}
