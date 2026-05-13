use super::*;
use crate::engine::auth::{read_auth, write_openai_auth, OpenAiApiKey};
use tempfile::TempDir;

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

    assert!(matches!(error, ConfigError::Auth(_)));
    assert_eq!(stored_key(&path).as_deref(), Some("sk-existing"));
}

#[test]
fn config_auth_rejects_unknown_provider_and_lists_valid_provider() {
    let dir = TempDir::new().unwrap();
    let path = auth_path(&dir);

    let error = run_auth("OpenAI", "sk-unused", &path).unwrap_err();

    assert_eq!(error.exit_code(), 2);
    assert!(matches!(error, ConfigError::UnknownProvider { .. }));
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
