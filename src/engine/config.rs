// Config read/write for bo
//
// Config lives at $HOME/.bo/config.json by default.
// All public functions accept an explicit path so callers (and tests) can
// redirect without touching global state.  Use config_path() to get the
// default location.
//
// Shape of config.json:
//
//   {
//     "compile_model": "gpt-4o",   // operator-level: spans all trees
//     "tree": {                     // active tree metadata
//       "output_dir": "/path/to/tree",
//       "name": "my-research",
//       "created_at": "2026-04-14T09:00:00Z"
//     }
//   }
//
// Top-level keys are operator/global settings.  Tree-specific fields live
// under "tree" so the boundary is explicit and multi-tree support can extend
// the shape without touching the global keys.

use crate::domain::tree::TreeConfig;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

// ── Config ──────────────────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    /// Active tree metadata.
    pub tree: TreeConfig,

    /// Model used by `bo compile`. Operator-level: applies across all trees.
    /// Defaults to "gpt-4o" when absent.
    #[serde(default)]
    pub compile_model: Option<String>,
}

impl Config {
    pub fn effective_compile_model(&self) -> &str {
        self.compile_model.as_deref().unwrap_or("gpt-4o")
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
