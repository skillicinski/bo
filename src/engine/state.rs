use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::path::Path;

/// ISO 8601 timestamp string.
type Timestamp = String;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TreeState {
    /// Leaf slug → ISO 8601 timestamp of when it was last included in a compile run.
    #[serde(default)]
    pub compiled_leaves: HashMap<String, Timestamp>,
}

#[derive(Debug)]
pub enum StateError {
    Io(io::Error),
}

impl fmt::Display for StateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StateError::Io(e) => write!(f, "state I/O error: {}", e),
        }
    }
}

impl From<io::Error> for StateError {
    fn from(e: io::Error) -> Self {
        StateError::Io(e)
    }
}

pub fn read_state(path: &Path) -> TreeState {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => TreeState::default(),
        Err(e) => {
            eprintln!("warning: failed to read state file: {}", e);
            TreeState::default()
        }
        Ok(contents) => match serde_json::from_str(&contents) {
            Ok(state) => state,
            Err(e) => {
                eprintln!("warning: malformed state file, ignoring: {}", e);
                TreeState::default()
            }
        },
    }
}

pub fn write_state(path: &Path, state: &TreeState) -> Result<(), StateError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(state).expect("TreeState serialization is infallible");
    std::fs::write(path, json)?;
    Ok(())
}

pub fn slug_from_filename(filename: &str) -> &str {
    filename.strip_suffix(".md").unwrap_or(filename)
}

#[cfg(test)]
#[path = "../tests/engine_state_tests.rs"]
mod tests;
