// Config read/write for bo
//
// Config lives at $HOME/.bo/config.json by default.
// All public functions accept an explicit path so callers (and tests) can
// redirect without touching global state.  Use config_path() to get the
// default location.
//
// Shape of config.json after `bo seed`:
//
//   {
//     "model": "gpt-4.1-mini",     // operator-level: spans all trees
//     "tree": {                     // active tree metadata
//       "output_dir": "/path/to/tree",
//       "name": "my-research",
//       "created_at": "2026-04-14T09:00:00Z"
//     }
//   }
//
// Config may also exist before `bo seed` with only operator-level keys, e.g.
// `{ "model": "gpt-4.1-mini" }`.

use crate::domain::tree::TreeConfig;
use crate::engine::llm::models::DEFAULT_MODEL;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

// ── Config ──────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Active tree metadata. Absent when config exists before `bo seed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<TreeConfig>,

    /// Global model used by LLM-backed stages. Defaults to `DEFAULT_MODEL` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Config {
    pub fn effective_model(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_MODEL)
    }

    pub fn into_seeded(self) -> Option<SeededConfig> {
        self.tree.map(|tree| SeededConfig {
            tree,
            model: self.model,
        })
    }
}

#[derive(Debug, Clone)]
pub struct SeededConfig {
    pub tree: TreeConfig,
    pub model: Option<String>,
}

impl SeededConfig {
    pub fn effective_model(&self) -> &str {
        self.model.as_deref().unwrap_or(DEFAULT_MODEL)
    }
}

#[derive(Debug)]
pub enum ConfigError {
    NotFound,
    Io(io::Error),
    Parse(serde_json::Error),
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConfigError::NotFound => write!(f, "config file not found"),
            ConfigError::Io(e) => write!(f, "config I/O error: {}", e),
            ConfigError::Parse(e) => write!(f, "config parse error: {}", e),
        }
    }
}

/// Returns the default path to the bo config file: $HOME/.bo/config.json.
pub fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".bo").join("config.json")
}

/// Read and deserialise the config from `path`.
/// Returns ConfigError::NotFound if the file does not exist.
pub fn read_config(path: &Path) -> Result<Config, ConfigError> {
    if !path.exists() {
        return Err(ConfigError::NotFound);
    }
    let contents = std::fs::read_to_string(path).map_err(ConfigError::Io)?;
    serde_json::from_str(&contents).map_err(ConfigError::Parse)
}

/// Serialise and write the config to `path`.
/// Creates the parent directory if it does not exist.
pub fn write_config(config: &Config, path: &Path) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(ConfigError::Io)?;
    }
    let json = serde_json::to_string_pretty(config).map_err(ConfigError::Parse)?;
    std::fs::write(path, json).map_err(ConfigError::Io)?;
    Ok(())
}

#[cfg(test)]
#[path = "../tests/engine_config_tests.rs"]
mod tests;
