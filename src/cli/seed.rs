use crate::domain::tree::TreeConfig;
use crate::engine::config::{self, Config, ConfigError};

use chrono::Utc;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct SeedResult {
    pub status: String,
    pub output_dir: String,
    pub tree_name: Option<String>,
}

#[derive(Debug)]
pub enum SeedError {
    ConfigRead(String),
    ConfigWrite(String),
    CreateOutputDir(String),
    CurrentDir(String),
}

impl std::fmt::Display for SeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConfigRead(msg) => write!(f, "{}", msg),
            Self::ConfigWrite(msg) => write!(f, "{}", msg),
            Self::CreateOutputDir(msg) => write!(f, "{}", msg),
            Self::CurrentDir(msg) => write!(f, "{}", msg),
        }
    }
}

pub fn seed(
    output_dir: PathBuf,
    name: Option<String>,
    config_path: &Path,
) -> Result<SeedResult, SeedError> {
    let output_dir = if output_dir.is_absolute() {
        output_dir
    } else {
        std::env::current_dir()
            .map_err(|error| SeedError::CurrentDir(format!("failed to get current dir: {error}")))?
            .join(&output_dir)
    };

    match config::read_config(config_path) {
        Ok(existing) => {
            return Ok(SeedResult {
                status: "already_seeded".to_string(),
                output_dir: path_string(&existing.tree.output_dir),
                tree_name: existing.tree.name,
            });
        }
        Err(ConfigError::NotFound) => {}
        Err(error) => {
            return Err(SeedError::ConfigRead(format!(
                "failed to read config: {}",
                error
            )));
        }
    }

    std::fs::create_dir_all(&output_dir).map_err(|error| {
        SeedError::CreateOutputDir(format!("failed to create output directory: {error}"))
    })?;

    let tree_name = name.or_else(|| {
        output_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    });

    let created_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    config::write_config(
        &Config {
            tree: TreeConfig {
                output_dir: output_dir.clone(),
                name: tree_name.clone(),
                created_at: Some(created_at),
            },
            compile_model: None,
        },
        config_path,
    )
    .map_err(|error| SeedError::ConfigWrite(format!("failed to write config: {error}")))?;

    Ok(SeedResult {
        status: "created".to_string(),
        output_dir: path_string(&output_dir),
        tree_name,
    })
}

pub fn render_human(result: &SeedResult) -> String {
    match result.status.as_str() {
        "already_seeded" => format!("bo has already been seeded at {}!", result.output_dir),
        _ => format!("seeded bo at {}", result.output_dir),
    }
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}

#[cfg(test)]
#[path = "../tests/cli_seed_tests.rs"]
mod tests;
