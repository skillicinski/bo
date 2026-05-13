use crate::engine::config::{self as engine_config, Config, ConfigError};
use crate::engine::llm::models::{is_supported_model, supported_model_ids};
use serde::Serialize;
use std::fmt;
use std::path::Path;

const MODEL_KEY: &str = "model";
const VALID_KEYS: &[&str] = &[MODEL_KEY];

#[derive(Debug, Clone, Serialize)]
pub struct ConfigCommandResult {
    pub action: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug)]
pub enum ConfigCommandError {
    UnknownKey { key: String },
    UnsupportedModel { model: String },
    Read(String),
    Write(String),
}

impl ConfigCommandError {
    pub fn exit_code(&self) -> i32 {
        match self {
            ConfigCommandError::UnknownKey { .. } | ConfigCommandError::UnsupportedModel { .. } => {
                2
            }
            ConfigCommandError::Read(_) | ConfigCommandError::Write(_) => 1,
        }
    }

    pub fn valid_keys(&self) -> Option<&'static [&'static str]> {
        match self {
            ConfigCommandError::UnknownKey { .. } => Some(VALID_KEYS),
            _ => None,
        }
    }

    pub fn supported_models(&self) -> Option<Vec<&'static str>> {
        match self {
            ConfigCommandError::UnsupportedModel { .. } => Some(supported_models()),
            _ => None,
        }
    }
}

impl fmt::Display for ConfigCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigCommandError::UnknownKey { key } => write!(
                f,
                "unknown config key '{}'; valid keys: {}",
                key,
                VALID_KEYS.join(", ")
            ),
            ConfigCommandError::UnsupportedModel { model } => write!(
                f,
                "unsupported model '{}'; supported models: {}",
                model,
                supported_models().join(", ")
            ),
            ConfigCommandError::Read(message) | ConfigCommandError::Write(message) => {
                write!(f, "{}", message)
            }
        }
    }
}

pub fn get(key: &str, config_path: &Path) -> Result<ConfigCommandResult, ConfigCommandError> {
    validate_key(key)?;
    let config = read_or_default(config_path)?;

    Ok(ConfigCommandResult {
        action: "get".to_string(),
        key: MODEL_KEY.to_string(),
        value: config.effective_model().to_string(),
    })
}

pub fn set(
    key: &str,
    value: &str,
    config_path: &Path,
) -> Result<ConfigCommandResult, ConfigCommandError> {
    validate_key(key)?;

    let model = value.trim().to_string();
    if !is_supported_model(&model) {
        return Err(ConfigCommandError::UnsupportedModel { model });
    }

    let mut config = read_or_default(config_path)?;
    config.model = Some(model.clone());

    engine_config::write_config(&config, config_path)
        .map_err(|error| ConfigCommandError::Write(format!("failed to write config: {}", error)))?;

    Ok(ConfigCommandResult {
        action: "set".to_string(),
        key: MODEL_KEY.to_string(),
        value: model,
    })
}

pub fn render_human(result: &ConfigCommandResult) -> String {
    match result.action.as_str() {
        "get" => format!("{}\n", result.value),
        "set" => format!("{} = {}\n", result.key, result.value),
        _ => format!("{}\n", result.value),
    }
}

fn validate_key(key: &str) -> Result<(), ConfigCommandError> {
    if key == MODEL_KEY {
        Ok(())
    } else {
        Err(ConfigCommandError::UnknownKey {
            key: key.to_string(),
        })
    }
}

fn read_or_default(config_path: &Path) -> Result<Config, ConfigCommandError> {
    match engine_config::read_config(config_path) {
        Ok(config) => Ok(config),
        Err(ConfigError::NotFound) => Ok(Config::default()),
        Err(error) => Err(ConfigCommandError::Read(format!(
            "failed to read config: {}",
            error
        ))),
    }
}

fn supported_models() -> Vec<&'static str> {
    supported_model_ids().collect()
}

#[cfg(test)]
#[path = "../tests/cli_config_tests.rs"]
mod tests;
