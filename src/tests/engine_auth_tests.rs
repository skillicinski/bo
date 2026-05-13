use super::*;
use serial_test::serial;
use tempfile::TempDir;

struct EnvGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, original }
    }

    fn unset(key: &'static str) -> Self {
        let original = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, original }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

fn auth_file_path(dir: &TempDir) -> std::path::PathBuf {
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
fn openai_key_accepts_non_empty_trimmed_input() {
    let key = OpenAiApiKey::new("  sk-test-value  ").unwrap();

    assert_eq!(key.as_str(), "sk-test-value");
}

#[test]
fn openai_key_rejects_empty_input() {
    let error = OpenAiApiKey::new("  ").unwrap_err();

    assert!(matches!(error, AuthError::EmptyApiKey));
}

#[test]
fn openai_key_formatting_is_redacted() {
    let key = OpenAiApiKey::new("sk-secret-value").unwrap();

    assert!(!format!("{key:?}").contains("sk-secret-value"));
    assert!(!format!("{key}").contains("sk-secret-value"));
    assert!(format!("{key:?}").contains("redacted"));
}

#[test]
#[serial]
fn auth_path_uses_home_bo_auth_json() {
    let home = TempDir::new().unwrap();
    let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());

    assert_eq!(auth_path(), home.path().join(".bo").join("auth.json"));
}

#[test]
fn write_then_read_round_trip_stores_openai_key() {
    let dir = TempDir::new().unwrap();
    let path = auth_file_path(&dir);
    let key = OpenAiApiKey::new("sk-round-trip").unwrap();

    write_openai_auth(&path, key).unwrap();

    assert_eq!(stored_key(&path).as_deref(), Some("sk-round-trip"));
    assert!(path.exists());
}

#[test]
fn write_openai_auth_overwrites_existing_key() {
    let dir = TempDir::new().unwrap();
    let path = auth_file_path(&dir);

    write_openai_auth(&path, OpenAiApiKey::new("sk-old").unwrap()).unwrap();
    write_openai_auth(&path, OpenAiApiKey::new("sk-new").unwrap()).unwrap();

    assert_eq!(stored_key(&path).as_deref(), Some("sk-new"));
}

#[cfg(unix)]
#[test]
fn write_openai_auth_applies_restrictive_unix_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let dir = TempDir::new().unwrap();
    let path = auth_file_path(&dir);

    write_openai_auth(&path, OpenAiApiKey::new("sk-permissions").unwrap()).unwrap();

    let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[test]
#[serial]
fn resolver_uses_environment_key_before_reading_stored_auth() {
    let dir = TempDir::new().unwrap();
    let path = auth_file_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{not json").unwrap();
    let _api_key_guard = EnvGuard::set("OPENAI_API_KEY", "  sk-env  ");

    let resolved = resolve_openai_api_key(&path).unwrap();

    assert_eq!(resolved.source, AuthSource::Environment);
    assert_eq!(resolved.api_key.as_str(), "sk-env");
}

#[test]
#[serial]
fn resolver_ignores_empty_environment_and_uses_stored_auth() {
    let dir = TempDir::new().unwrap();
    let path = auth_file_path(&dir);
    write_openai_auth(&path, OpenAiApiKey::new("sk-stored").unwrap()).unwrap();
    let _api_key_guard = EnvGuard::set("OPENAI_API_KEY", "   ");

    let resolved = resolve_openai_api_key(&path).unwrap();

    assert_eq!(resolved.source, AuthSource::StoredAuth);
    assert_eq!(resolved.api_key.as_str(), "sk-stored");
}

#[test]
#[serial]
fn resolver_reports_missing_auth_with_setup_message() {
    let dir = TempDir::new().unwrap();
    let path = auth_file_path(&dir);
    let _api_key_guard = EnvGuard::unset("OPENAI_API_KEY");

    let error = resolve_openai_api_key(&path).unwrap_err();

    assert!(matches!(error, AuthResolutionError::Missing));
    assert_eq!(error.to_string(), MISSING_OPENAI_AUTH_MESSAGE);
}

#[test]
#[serial]
fn resolver_reports_malformed_auth_without_file_contents() {
    let dir = TempDir::new().unwrap();
    let path = auth_file_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "sk-leak-marker").unwrap();
    let _api_key_guard = EnvGuard::unset("OPENAI_API_KEY");

    let error = resolve_openai_api_key(&path).unwrap_err();
    let message = error.to_string();

    assert!(matches!(error, AuthResolutionError::Read(_)));
    assert!(!message.contains("sk-leak-marker"));
}

#[test]
#[serial]
fn resolver_treats_missing_provider_or_key_as_missing_auth() {
    let dir = TempDir::new().unwrap();
    let path = auth_file_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let _api_key_guard = EnvGuard::unset("OPENAI_API_KEY");

    std::fs::write(&path, r#"{"providers":{}}"#).unwrap();
    assert!(matches!(
        resolve_openai_api_key(&path).unwrap_err(),
        AuthResolutionError::Missing
    ));

    std::fs::write(&path, r#"{"providers":{"openai":{}}}"#).unwrap();
    assert!(matches!(
        resolve_openai_api_key(&path).unwrap_err(),
        AuthResolutionError::Missing
    ));
}

#[test]
#[serial]
fn resolver_rejects_empty_stored_key() {
    let dir = TempDir::new().unwrap();
    let path = auth_file_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, r#"{"providers":{"openai":{"api_key":"   "}}}"#).unwrap();
    let _api_key_guard = EnvGuard::unset("OPENAI_API_KEY");

    let error = resolve_openai_api_key(&path).unwrap_err();

    assert!(matches!(error, AuthResolutionError::Read(_)));
}
