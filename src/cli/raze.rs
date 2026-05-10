use crate::cli::json::JsonWarning;
use crate::domain::index;

use serde::Serialize;
use serde_json::json;
use std::io::ErrorKind as IoErrorKind;
use std::path::{Component, Path};

#[derive(Debug, Clone, Serialize)]
pub struct RazeResult {
    pub deleted_files: usize,
    pub deleted_index: bool,
    pub removed_output_dir: bool,
    pub output_dir_left_in_place: bool,
    pub deleted_config: bool,
    pub output_dir: String,
    pub config_path: String,
}

#[derive(Debug)]
pub struct RazeOutput {
    pub result: RazeResult,
    pub warnings: Vec<JsonWarning>,
}

#[derive(Debug)]
pub enum RazeError {
    Io(String),
}

impl std::fmt::Display for RazeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "{}", msg),
        }
    }
}

pub fn raze(output_dir: &Path, config_path: &Path) -> Result<RazeOutput, RazeError> {
    let index_path = output_dir.join("index.jsonl");
    let entries = index::read_index(&index_path)
        .map_err(|error| RazeError::Io(format!("failed to read index: {error}")))?;

    let mut deleted_files = 0usize;
    let mut warnings = Vec::new();

    for entry in &entries {
        if is_suspicious_relative_path(&entry.file) {
            warnings.push(JsonWarning::with_details(
                "suspicious_ledger_entry",
                format!("skipping ledger entry with suspicious path: {}", entry.file),
                json!({ "file": entry.file }),
            ));
            continue;
        }

        let resolved = output_dir.join(&entry.file);

        match std::fs::remove_file(&resolved) {
            Ok(()) => deleted_files += 1,
            Err(error) if error.kind() == IoErrorKind::NotFound => {}
            Err(error) => {
                return Err(RazeError::Io(format!(
                    "failed to delete {}: {}",
                    resolved.display(),
                    error
                )));
            }
        }
    }

    let deleted_index = match std::fs::remove_file(&index_path) {
        Ok(()) => true,
        Err(error) if error.kind() == IoErrorKind::NotFound => false,
        Err(error) => {
            return Err(RazeError::Io(format!("failed to delete ledger: {}", error)));
        }
    };

    let (removed_output_dir, output_dir_left_in_place) = match std::fs::remove_dir(output_dir) {
        Ok(()) => (true, false),
        Err(error)
            if error.kind() == IoErrorKind::DirectoryNotEmpty
                || error.kind() == IoErrorKind::NotFound =>
        {
            (false, true)
        }
        Err(error) => {
            return Err(RazeError::Io(format!(
                "failed to remove output directory: {}",
                error
            )));
        }
    };

    let deleted_config = match std::fs::remove_file(config_path) {
        Ok(()) => true,
        Err(error) if error.kind() == IoErrorKind::NotFound => false,
        Err(error) => {
            return Err(RazeError::Io(format!("failed to delete config: {}", error)));
        }
    };

    Ok(RazeOutput {
        result: RazeResult {
            deleted_files,
            deleted_index,
            removed_output_dir,
            output_dir_left_in_place,
            deleted_config,
            output_dir: path_string(output_dir),
            config_path: path_string(config_path),
        },
        warnings,
    })
}

pub fn render_human(result: &RazeResult) -> String {
    let mut out = format!("deleted {} markdown file(s)\n", result.deleted_files);

    if result.deleted_index {
        out.push_str("deleted index\n");
    }

    if result.removed_output_dir {
        out.push_str(&format!("removed output directory {}\n", result.output_dir));
    } else if result.output_dir_left_in_place {
        out.push_str(&format!(
            "output directory left in place (not empty or already absent): {}\n",
            result.output_dir
        ));
    }

    if result.deleted_config {
        out.push_str("deleted config\n");
    }

    out
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn is_suspicious_relative_path(file: &str) -> bool {
    let relative = Path::new(file);
    relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
}

#[cfg(test)]
#[path = "../tests/cli_raze_tests.rs"]
mod tests;
