use super::*;
use crate::domain::tree::TreeConfig;
use crate::engine::auth::{read_auth, write_openai_auth, OpenAiApiKey};
use crate::engine::config::{self as engine_config, Config};
use std::path::PathBuf;
use tempfile::TempDir;

// ── auth tests ───────────────────────────────────────────────────────────────

fn auth_path(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join(".bo").join("auth.json")
}

fn stored_key(path: &std::path::Path) -> Option<String> {
    read_auth(path)
        .ok()?
        .providers
        .openai?
        .api_key
        .map(|key| key.as_str().to_string())
}

#[test]
fn config_auth_stores_openai_key_and_returns_non_secret_status() {
    let dir = TempDir::new().unwrap();
    let path = auth_path(&dir);

    let output = run_auth("openai", "sk-configured", &path).unwrap();

    assert_eq!(stored_key(&path).as_deref(), Some("sk-configured"));
    assert_eq!(output.result.status, "ok");
    assert_eq!(output.result.provider, "openai");
    assert_eq!(output.result.auth, "configured");
    assert!(!serde_json::to_string(&output.result)
        .unwrap()
        .contains("sk-configured"));
}

#[test]
fn config_auth_overwrites_existing_openai_key() {
    let dir = TempDir::new().unwrap();
    let path = auth_path(&dir);

    run_auth("openai", "sk-old", &path).unwrap();
    run_auth("openai", "sk-new", &path).unwrap();

    assert_eq!(stored_key(&path).as_deref(), Some("sk-new"));
}

#[test]
fn config_auth_rejects_empty_key_without_overwriting_existing_key() {
    let dir = TempDir::new().unwrap();
    let path = auth_path(&dir);
    write_openai_auth(&path, OpenAiApiKey::new("sk-existing").unwrap()).unwrap();

    let error = run_auth("openai", "   ", &path).unwrap_err();

    assert!(matches!(error, ConfigAuthError::Auth(_)));
    assert_eq!(stored_key(&path).as_deref(), Some("sk-existing"));
}

#[test]
fn config_auth_rejects_unknown_provider_and_lists_valid_provider() {
    let dir = TempDir::new().unwrap();
    let path = auth_path(&dir);

    let error = run_auth("OpenAI", "sk-unused", &path).unwrap_err();

    assert_eq!(error.exit_code(), 2);
    assert!(matches!(error, ConfigAuthError::UnknownProvider { .. }));
    assert!(error.to_string().contains("openai"));
    assert!(!path.exists());
}

#[test]
fn config_auth_error_formatting_does_not_include_key_input() {
    let dir = TempDir::new().unwrap();
    let path = auth_path(&dir);

    let error = run_auth("unknown", "sk-should-not-appear", &path).unwrap_err();

    assert!(!error.to_string().contains("sk-should-not-appear"));
    assert!(!error.details().to_string().contains("sk-should-not-appear"));
}

#[test]
fn render_auth_human_contains_provider_but_not_secret() {
    let result = ConfigAuthResult {
        status: "ok".to_string(),
        provider: "openai".to_string(),
        auth: "configured".to_string(),
    };

    let rendered = render_auth_human(&result);

    assert!(rendered.contains("openai"));
    assert!(!rendered.contains("sk-"));
}

// ── config set/get tests ─────────────────────────────────────────────────────

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
