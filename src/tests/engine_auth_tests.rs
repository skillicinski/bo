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

fn write_auth_json(auth_dir: &TempDir, key_name: &str, value: &str) {
    let auth_dir_path = auth_dir.path().join(".bo");
    std::fs::create_dir_all(&auth_dir_path).unwrap();
    let content = format!(r#"{{"{}": "{}"}}"#, key_name, value);
    std::fs::write(auth_dir_path.join("auth.json"), content).unwrap();
}

#[test]
#[serial]
fn test_env_var_openai() {
    let dir = TempDir::new().unwrap();
    let _guard = EnvGuard::set("OPENAI_API_KEY", "sk-openai-env");
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());

    let result = resolve_api_key(Provider::OpenAI).unwrap();
    assert_eq!(result, "sk-openai-env");
}

#[test]
#[serial]
fn test_env_var_deepseek() {
    let dir = TempDir::new().unwrap();
    let _guard = EnvGuard::set("DEEPSEEK_API_KEY", "sk-deepseek-env");
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());

    let result = resolve_api_key(Provider::Deepseek).unwrap();
    assert_eq!(result, "sk-deepseek-env");
}

#[test]
#[serial]
fn test_env_var_trims_whitespace() {
    let dir = TempDir::new().unwrap();
    let _guard = EnvGuard::set("OPENAI_API_KEY", "  sk-openai-env  ");
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());

    let result = resolve_api_key(Provider::OpenAI).unwrap();
    assert_eq!(result, "sk-openai-env");
}

#[test]
#[serial]
fn test_empty_env_var_falls_through_to_auth_json() {
    let dir = TempDir::new().unwrap();
    let _guard = EnvGuard::set("OPENAI_API_KEY", "   ");
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());
    write_auth_json(&dir, "openai_api_key", "sk-json-fallback");

    let result = resolve_api_key(Provider::OpenAI).unwrap();
    assert_eq!(result, "sk-json-fallback");
}

#[test]
#[serial]
fn test_auth_json_fallback_openai() {
    let dir = TempDir::new().unwrap();
    let _guard = EnvGuard::unset("OPENAI_API_KEY");
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());
    write_auth_json(&dir, "openai_api_key", "sk-json-key");

    let result = resolve_api_key(Provider::OpenAI).unwrap();
    assert_eq!(result, "sk-json-key");
}

#[test]
#[serial]
fn test_auth_json_fallback_deepseek() {
    let dir = TempDir::new().unwrap();
    let _guard = EnvGuard::unset("DEEPSEEK_API_KEY");
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());
    write_auth_json(&dir, "deepseek_api_key", "sk-deepseek-json");

    let result = resolve_api_key(Provider::Deepseek).unwrap();
    assert_eq!(result, "sk-deepseek-json");
}

#[test]
#[serial]
fn test_missing_key_error() {
    let dir = TempDir::new().unwrap();
    let _openai_guard = EnvGuard::unset("OPENAI_API_KEY");
    let _deepseek_guard = EnvGuard::unset("DEEPSEEK_API_KEY");
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());

    let error = resolve_api_key(Provider::OpenAI).unwrap_err();
    assert!(matches!(error, AuthError::Missing { provider } if provider == Provider::OpenAI));
    assert_eq!(
        error.to_string(),
        "OPENAI_API_KEY environment variable not set"
    );
}

#[test]
#[serial]
fn test_env_takes_precedence_over_auth_json() {
    let dir = TempDir::new().unwrap();
    let _guard = EnvGuard::set("OPENAI_API_KEY", "sk-env-wins");
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());
    write_auth_json(&dir, "openai_api_key", "sk-should-not-be-used");

    let result = resolve_api_key(Provider::OpenAI).unwrap();
    assert_eq!(result, "sk-env-wins");
}

#[test]
#[serial]
fn test_auth_path_default() {
    let dir = TempDir::new().unwrap();
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());

    assert_eq!(auth_path(), dir.path().join(".bo").join("auth.json"));
}

#[test]
#[serial]
fn test_display_missing_openai() {
    let error = AuthError::Missing {
        provider: Provider::OpenAI,
    };
    assert_eq!(
        error.to_string(),
        "OPENAI_API_KEY environment variable not set"
    );
}

#[test]
#[serial]
fn test_display_missing_deepseek() {
    let error = AuthError::Missing {
        provider: Provider::Deepseek,
    };
    assert_eq!(
        error.to_string(),
        "DEEPSEEK_API_KEY environment variable not set"
    );
}

#[test]
#[serial]
fn test_display_missing_google() {
    let error = AuthError::Missing {
        provider: Provider::Google,
    };
    assert_eq!(
        error.to_string(),
        "GEMINI_API_KEY environment variable not set"
    );
}

#[test]
#[serial]
fn test_env_var_google() {
    let dir = TempDir::new().unwrap();
    let _guard = EnvGuard::set("GEMINI_API_KEY", "gemini-env-key");
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());

    let result = resolve_api_key(Provider::Google).unwrap();
    assert_eq!(result, "gemini-env-key");
}

#[test]
#[serial]
fn test_auth_json_fallback_google() {
    let dir = TempDir::new().unwrap();
    let _guard = EnvGuard::unset("GEMINI_API_KEY");
    let _home_guard = EnvGuard::set("HOME", dir.path().to_str().unwrap());
    write_auth_json(&dir, "google_api_key", "gemini-json-key");

    let result = resolve_api_key(Provider::Google).unwrap();
    assert_eq!(result, "gemini-json-key");
}
