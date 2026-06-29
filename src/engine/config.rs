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
//     "provider": "openai",          // operator-level: spans all trees
//     "model": "gpt-4.1-mini",      // operator-level: spans all trees
//     "compile_model": "gpt-4.1",   // optional model used by compile
//     "tree": {                      // active tree metadata
//       "path": "/path/to/tree",
//       "name": "my-research",
//       "created_at": "2026-04-14T09:00:00.000Z"
//     }
//   }
//
// Config may also exist before `bo seed` with only operator-level keys, e.g.
// `{ "model": "gpt-4.1-mini" }`.

use crate::domain::tree::{Tree, TreeConfig};
use crate::engine::llm::model::Model;
use crate::engine::llm::Provider;
use crate::engine::llm::UnsupportedModel;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

// ── Config ──────────────────────────────────────────────────────────────────────────

fn default_provider() -> Provider {
    Provider::OpenAI
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// LLM provider. Defaults to OpenAI.
    #[serde(default = "default_provider")]
    pub provider: Provider,

    /// Global model used by LLM-backed stages.
    #[serde(default)]
    pub model: String,

    /// Optional model used by compile. Falls back to `model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compile_model: Option<String>,

    /// Active tree metadata. Absent when config exists before `bo seed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<TreeConfig>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            provider: Provider::OpenAI,
            model: String::new(),
            compile_model: None,
            tree: None,
        }
    }
}

impl Config {
    pub fn effective_model(&self) -> Result<Model, UnsupportedModel> {
        Model::parse(&self.model, self.provider)
    }

    pub fn effective_compile_model(&self) -> Result<Model, UnsupportedModel> {
        let model_id = self.compile_model.as_deref().unwrap_or(&self.model);
        Model::parse(model_id, self.provider)
    }

    pub fn into_seeded(self) -> Option<SeededConfig> {
        self.tree
            .map(|tree_cfg| SeededConfig::new(Config { tree: None, ..self }, tree_cfg))
    }
}

#[derive(Debug, Clone)]
pub struct SeededConfig {
    pub config: Config,
    tree_cfg: TreeConfig,
}

impl SeededConfig {
    pub fn new(mut config: Config, tree_cfg: TreeConfig) -> Self {
        config.tree = None;
        Self { config, tree_cfg }
    }

    pub fn tree(&self) -> Tree {
        Tree::from_config(&self.tree_cfg)
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
    let home = std::env::var("HOME")
        .expect("$HOME environment variable must be set to locate bo configuration");
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
