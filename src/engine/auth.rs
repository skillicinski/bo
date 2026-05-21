use crate::engine::llm::Provider;
use std::fmt;
use std::io;
use std::path::PathBuf;

#[derive(Debug)]
pub enum AuthError {
    Missing { provider: Provider },
    Io(io::Error),
    Parse(serde_json::Error),
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AuthError::Missing { provider } => {
                let env_var = match provider {
                    Provider::OpenAI => "OPENAI_API_KEY",
                    Provider::Deepseek => "DEEPSEEK_API_KEY",
                };
                write!(f, "{} environment variable not set", env_var)
            }
            AuthError::Io(e) => write!(f, "auth I/O error: {}", e),
            AuthError::Parse(e) => write!(f, "auth parse error: {}", e),
        }
    }
}

pub fn auth_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".bo").join("auth.json")
}

/// Resolve the API key for a provider.
/// 1. Check env var (OPENAI_API_KEY or DEEPSEEK_API_KEY)
/// 2. Fall back to ~/.bo/auth.json with flat keys `openai_api_key` / `deepseek_api_key`
/// 3. Error with missing key message
pub fn resolve_api_key(provider: Provider) -> Result<String, AuthError> {
    let env_var = match provider {
        Provider::OpenAI => "OPENAI_API_KEY",
        Provider::Deepseek => "DEEPSEEK_API_KEY",
    };

    // 1. Check env var
    match std::env::var(env_var) {
        Ok(val) if !val.trim().is_empty() => return Ok(val.trim().to_string()),
        _ => {} // Empty or not set — fall through
    }

    // 2. Fall back to auth.json
    let path = auth_path();
    if path.exists() {
        let contents = std::fs::read_to_string(&path).map_err(AuthError::Io)?;
        let parsed: serde_json::Value =
            serde_json::from_str(&contents).map_err(AuthError::Parse)?;
        let key_name = match provider {
            Provider::OpenAI => "openai_api_key",
            Provider::Deepseek => "deepseek_api_key",
        };
        if let Some(key) = parsed.get(key_name).and_then(|v| v.as_str()) {
            if !key.trim().is_empty() {
                return Ok(key.to_string());
            }
        }
    }

    // 3. Missing
    Err(AuthError::Missing { provider })
}

#[cfg(test)]
#[path = "../tests/engine_auth_tests.rs"]
mod tests;
