// Flag-based config writing for bo.
//
// Replaces the old get/set/auth subcommands with a flag-driven interface:
//   bo config --provider deepseek --model deepseek-v4-flash
//
// Reads existing config, applies the requested changes, validates model
// compatibility with the (new or existing) provider, and writes back.

use crate::engine::config::{self as engine_config, Config, ConfigError};
use crate::engine::llm::models::{self, models_for};
use crate::engine::llm::{Provider, ALL_PROVIDERS};
use serde::Serialize;
use serde_json::json;
use std::fmt;
use std::path::Path;

// ── Options ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WriteConfigOptions {
    pub provider: Option<Provider>,
    pub model: Option<String>,
    pub compile_model: Option<String>,
}

// ── Result types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct ConfigWriteResult {
    pub status: String,
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compile_model: Option<String>,
}

#[derive(Debug)]
pub enum ConfigWriteError {
    UnknownProvider { raw: String },
    UnsupportedModel { model: String, provider: Provider },
    Read(String),
    Write(String),
}

impl ConfigWriteError {
    pub fn exit_code(&self) -> i32 {
        match self {
            ConfigWriteError::UnknownProvider { .. } => 2,
            ConfigWriteError::UnsupportedModel { .. } => 2,
            ConfigWriteError::Read(_) => 1,
            ConfigWriteError::Write(_) => 1,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            ConfigWriteError::UnknownProvider { .. } => "usage_error",
            ConfigWriteError::UnsupportedModel { .. } => "usage_error",
            ConfigWriteError::Read(_) => "io_error",
            ConfigWriteError::Write(_) => "io_error",
        }
    }

    pub fn details(&self) -> serde_json::Value {
        match self {
            ConfigWriteError::UnknownProvider { raw } => json!({
                "provider": raw,
                "valid_providers": ALL_PROVIDERS,
            }),
            ConfigWriteError::UnsupportedModel { model, provider } => json!({
                "model": model,
                "provider": provider.to_string(),
                "supported_models": models_for(*provider).iter().map(|m| m.id).collect::<Vec<_>>(),
            }),
            _ => json!({}),
        }
    }
}

impl fmt::Display for ConfigWriteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigWriteError::UnknownProvider { raw } => write!(
                f,
                "unknown provider '{}'; valid providers: {}",
                raw,
                ALL_PROVIDERS.join(", ")
            ),
            ConfigWriteError::UnsupportedModel { model, provider } => {
                let supported: Vec<&str> = models_for(*provider).iter().map(|m| m.id).collect();
                write!(
                    f,
                    "unsupported model '{}' for provider '{}'; supported models: {}",
                    model,
                    provider,
                    supported.join(", ")
                )
            }
            ConfigWriteError::Read(msg) => write!(f, "failed to read config: {}", msg),
            ConfigWriteError::Write(msg) => write!(f, "failed to write config: {}", msg),
        }
    }
}

impl std::error::Error for ConfigWriteError {}

// ── Write logic ──────────────────────────────────────────────────────────────

pub fn write_config(
    options: WriteConfigOptions,
    config_path: &Path,
) -> Result<ConfigWriteResult, ConfigWriteError> {
    // Read existing config, or start with defaults
    let mut config = match engine_config::read_config(config_path) {
        Ok(c) => c,
        Err(ConfigError::NotFound) => Config::default(),
        Err(e) => {
            return Err(ConfigWriteError::Read(format!(
                "failed to read config: {}",
                e
            )))
        }
    };

    // Determine effective provider for model validation:
    // Use the new provider if --provider was set, else the existing config's provider
    let effective_provider = options.provider.unwrap_or(config.provider);

    // Apply provider if specified
    if let Some(provider) = options.provider {
        config.provider = provider;
    }

    // Validate and apply model if specified
    if let Some(ref model) = options.model {
        let trimmed = model.trim().to_string();
        if !models::is_supported_model(effective_provider, &trimmed) {
            return Err(ConfigWriteError::UnsupportedModel {
                model: trimmed,
                provider: effective_provider,
            });
        }
        config.model = trimmed;
    }

    // Validate and apply compile_model if specified
    if let Some(ref cm) = options.compile_model {
        let trimmed = cm.trim().to_string();
        if !models::is_supported_model(effective_provider, &trimmed) {
            return Err(ConfigWriteError::UnsupportedModel {
                model: trimmed,
                provider: effective_provider,
            });
        }
        config.compile_model = Some(trimmed);
    }

    // Write config
    engine_config::write_config(&config, config_path)
        .map_err(|e| ConfigWriteError::Write(format!("failed to write config: {}", e)))?;

    Ok(ConfigWriteResult {
        status: "ok".to_string(),
        provider: config.provider.to_string(),
        model: config.model,
        compile_model: config.compile_model,
    })
}

// ── Human output ─────────────────────────────────────────────────────────────

pub fn render_human(result: &ConfigWriteResult) -> String {
    let mut lines = vec![
        format!("provider: {}", result.provider),
        format!("model: {}", result.model),
    ];
    if let Some(ref cm) = result.compile_model {
        lines.push(format!("compile_model: {}", cm));
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
#[path = "../tests/cli_config_tests.rs"]
mod tests;
