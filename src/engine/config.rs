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
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_config_path(dir: &TempDir) -> PathBuf {
        dir.path().join(".bo").join("config.json")
    }

    fn make_config(output_dir: &str) -> Config {
        Config {
            tree: TreeConfig {
                output_dir: PathBuf::from(output_dir),
                name: None,
                created_at: None,
            },
            compile_model: None,
        }
    }

    #[test]
    fn write_then_read_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = temp_config_path(&dir);

        write_config(&make_config("/tmp/my-tree"), &path).unwrap();

        let loaded = read_config(&path).unwrap();
        assert_eq!(loaded.tree.output_dir, PathBuf::from("/tmp/my-tree"));
    }

    #[test]
    fn written_file_is_valid_json_with_tree_key() {
        let dir = TempDir::new().unwrap();
        let path = temp_config_path(&dir);

        write_config(&make_config("/some/path"), &path).unwrap();

        let contents = std::fs::read_to_string(&path).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
        assert_eq!(parsed["tree"]["output_dir"], "/some/path");
    }

    #[test]
    fn read_nonexistent_returns_not_found() {
        let dir = TempDir::new().unwrap();
        let path = temp_config_path(&dir);

        let err = read_config(&path).unwrap_err();
        assert!(matches!(err, ConfigError::NotFound));
    }

    #[test]
    fn read_malformed_json_returns_parse_error() {
        let dir = TempDir::new().unwrap();
        let path = temp_config_path(&dir);

        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "not json at all").unwrap();

        let err = read_config(&path).unwrap_err();
        assert!(matches!(err, ConfigError::Parse(_)));
    }

    #[test]
    fn compile_model_roundtrip_with_value() {
        let dir = TempDir::new().unwrap();
        let path = temp_config_path(&dir);

        let config = Config {
            tree: TreeConfig {
                output_dir: PathBuf::from("/tmp/bo"),
                name: None,
                created_at: None,
            },
            compile_model: Some("gpt-4o-mini".to_string()),
        };
        write_config(&config, &path).unwrap();

        let loaded = read_config(&path).unwrap();
        assert_eq!(loaded.compile_model.as_deref(), Some("gpt-4o-mini"));
        assert_eq!(loaded.effective_compile_model(), "gpt-4o-mini");
    }

    #[test]
    fn name_and_created_at_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = temp_config_path(&dir);

        let config = Config {
            tree: TreeConfig {
                output_dir: PathBuf::from("/tmp/bo"),
                name: Some("my-research".to_string()),
                created_at: Some("2026-04-14T09:00:00Z".to_string()),
            },
            compile_model: None,
        };
        write_config(&config, &path).unwrap();

        let loaded = read_config(&path).unwrap();
        assert_eq!(loaded.tree.name.as_deref(), Some("my-research"));
        assert_eq!(
            loaded.tree.created_at.as_deref(),
            Some("2026-04-14T09:00:00Z")
        );
    }

    #[test]
    fn compile_model_absent_uses_default() {
        let dir = TempDir::new().unwrap();
        let path = temp_config_path(&dir);

        // Write JSON without compile_model field
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"tree":{"output_dir":"/tmp/bo"}}"#).unwrap();

        let loaded = read_config(&path).unwrap();
        assert!(loaded.compile_model.is_none());
        assert_eq!(loaded.effective_compile_model(), "gpt-4o");
    }

    #[test]
    fn effective_compile_model_returns_stored_value_when_set() {
        let cfg = Config {
            tree: TreeConfig {
                output_dir: PathBuf::from("/tmp/bo"),
                name: None,
                created_at: None,
            },
            compile_model: Some("claude-3-5-sonnet".to_string()),
        };
        assert_eq!(cfg.effective_compile_model(), "claude-3-5-sonnet");
    }
}
