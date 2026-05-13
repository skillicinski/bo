use crate::cli::json::JsonWarning;
use crate::engine::auth::{self, AuthError, OpenAiApiKey};
use serde::Serialize;
use serde_json::json;
use std::fmt;
use std::path::Path;

pub const OPENAI_PROVIDER: &str = "openai";
pub const VALID_PROVIDERS: &[&str] = &[OPENAI_PROVIDER];

#[derive(Debug, Clone, Serialize)]
pub struct ConfigAuthResult {
    pub status: String,
    pub provider: String,
    pub auth: String,
}

#[derive(Debug)]
pub struct ConfigAuthOutput {
    pub result: ConfigAuthResult,
    pub warnings: Vec<JsonWarning>,
}

#[derive(Debug)]
pub enum ConfigError {
    UnknownProvider { provider: String },
    Auth(AuthError),
}

impl ConfigError {
    pub fn exit_code(&self) -> i32 {
        match self {
            ConfigError::UnknownProvider { .. } => 2,
            ConfigError::Auth(_) => 1,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            ConfigError::UnknownProvider { .. } => "usage_error",
            ConfigError::Auth(AuthError::EmptyApiKey) => "validation_error",
            ConfigError::Auth(AuthError::Io(_)) => "io_error",
            ConfigError::Auth(AuthError::Parse(_)) => "auth_error",
            ConfigError::Auth(AuthError::NotFound) => "auth_error",
        }
    }

    pub fn details(&self) -> serde_json::Value {
        match self {
            ConfigError::UnknownProvider { .. } => {
                json!({ "valid_providers": VALID_PROVIDERS, "exit_code": self.exit_code() })
            }
            ConfigError::Auth(_) => json!({}),
        }
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::UnknownProvider { provider } => write!(
                f,
                "unknown provider '{}'; valid providers: {}",
                provider,
                VALID_PROVIDERS.join(", ")
            ),
            ConfigError::Auth(error) => write!(f, "{error}"),
        }
    }
}

pub fn validate_provider(provider: &str) -> Result<(), ConfigError> {
    if provider == OPENAI_PROVIDER {
        Ok(())
    } else {
        Err(ConfigError::UnknownProvider {
            provider: provider.to_string(),
        })
    }
}

pub fn run_auth(
    provider: &str,
    raw_api_key: impl Into<String>,
    auth_path: &Path,
) -> Result<ConfigAuthOutput, ConfigError> {
    validate_provider(provider)?;

    let api_key = OpenAiApiKey::new(raw_api_key.into()).map_err(ConfigError::Auth)?;
    let outcome = auth::write_openai_auth(auth_path, api_key).map_err(ConfigError::Auth)?;

    let warnings = outcome
        .permission_warning
        .into_iter()
        .map(|warning| JsonWarning::new("auth_permissions_not_restricted", warning.message))
        .collect();

    Ok(ConfigAuthOutput {
        result: ConfigAuthResult {
            status: "ok".to_string(),
            provider: OPENAI_PROVIDER.to_string(),
            auth: "configured".to_string(),
        },
        warnings,
    })
}

pub fn render_auth_human(result: &ConfigAuthResult) -> String {
    format!("{} auth configured\n", result.provider)
}

#[cfg(test)]
#[path = "../tests/cli_config_tests.rs"]
mod tests;
