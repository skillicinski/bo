use super::*;
use crate::domain::tree::TreeConfig;
use std::path::PathBuf;
use tempfile::TempDir;

fn temp_config_path(dir: &TempDir) -> PathBuf {
    dir.path().join(".bo").join("config.json")
}

fn seeded_config() -> Config {
    Config {
        tree: Some(TreeConfig {
            output_dir: PathBuf::from("/tmp/tree"),
            name: Some("tree".to_string()),
            created_at: Some("2026-05-12T00:00:00Z".to_string()),
        }),
        model: None,
    }
}

#[test]
fn get_absent_config_returns_default_model() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let result = get("model", &path).unwrap();

    assert_eq!(result.action, "get");
    assert_eq!(result.key, "model");
    assert_eq!(result.value, "gpt-4o");
    assert!(!path.exists());
}

#[test]
fn set_creates_config() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let result = set("model", "gpt-4.1-mini", &path).unwrap();

    assert_eq!(result.action, "set");
    assert_eq!(result.value, "gpt-4.1-mini");
    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.model.as_deref(), Some("gpt-4.1-mini"));
    assert!(loaded.tree.is_none());
}

#[test]
fn set_trims_model_value() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    set("model", " gpt-4.1-mini ", &path).unwrap();

    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.model.as_deref(), Some("gpt-4.1-mini"));
}

#[test]
fn set_preserves_tree_metadata() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);
    engine_config::write_config(&seeded_config(), &path).unwrap();

    set("model", "gpt-4.1-mini", &path).unwrap();

    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.model.as_deref(), Some("gpt-4.1-mini"));
    let tree = loaded.tree.unwrap();
    assert_eq!(tree.output_dir, PathBuf::from("/tmp/tree"));
    assert_eq!(tree.name.as_deref(), Some("tree"));
}

#[test]
fn unknown_get_key_is_usage_error() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let err = get("query_model", &path).unwrap_err();

    assert_eq!(err.exit_code(), 2);
    assert!(matches!(err, ConfigCommandError::UnknownKey { .. }));
    assert_eq!(err.valid_keys().unwrap(), &["model"]);
}

#[test]
fn unknown_set_key_is_usage_error_before_model_validation() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let err = set("query_model", "unknown-model", &path).unwrap_err();

    assert_eq!(err.exit_code(), 2);
    assert!(matches!(err, ConfigCommandError::UnknownKey { .. }));
}

#[test]
fn unsupported_model_is_usage_error() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let err = set("model", "unknown-model", &path).unwrap_err();

    assert_eq!(err.exit_code(), 2);
    assert!(matches!(err, ConfigCommandError::UnsupportedModel { .. }));
    assert!(err.supported_models().unwrap().contains(&"gpt-4.1-mini"));
}

#[test]
fn malformed_config_is_not_overwritten_by_set() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not json").unwrap();

    let err = set("model", "gpt-4.1-mini", &path).unwrap_err();

    assert_eq!(err.exit_code(), 1);
    assert!(matches!(err, ConfigCommandError::Read(_)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
}

#[test]
fn render_get_human_is_shell_friendly() {
    let rendered = render_human(&ConfigCommandResult {
        action: "get".to_string(),
        key: "model".to_string(),
        value: "gpt-4.1-mini".to_string(),
    });

    assert_eq!(rendered, "gpt-4.1-mini\n");
}

#[test]
fn render_set_human_is_concise() {
    let rendered = render_human(&ConfigCommandResult {
        action: "set".to_string(),
        key: "model".to_string(),
        value: "gpt-4.1-mini".to_string(),
    });

    assert_eq!(rendered, "model = gpt-4.1-mini\n");
}
