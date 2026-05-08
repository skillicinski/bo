use std::fs;
use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::engine::agent::{AgentError, Tool};

/// Reads a single leaf file by filename. Read-only, reusable by any agent.
pub struct ReadLeafTool {
    output_dir: PathBuf,
}

impl ReadLeafTool {
    pub fn new(output_dir: PathBuf) -> Self {
        Self { output_dir }
    }
}

#[async_trait]
impl Tool for ReadLeafTool {
    fn name(&self) -> &'static str {
        "read_leaf"
    }

    fn description(&self) -> &'static str {
        "Read the full content of a leaf document (frontmatter + body). \
         Use the 'file' values returned by list_index as the filename argument. \
         Returns the markdown content of the document."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "The leaf filename (with .md extension) as returned by list_index"
                }
            },
            "required": ["filename"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        let filename = match args.get("filename").and_then(|v| v.as_str()) {
            Some(f) => f.to_string(),
            None => return Ok("error: missing 'filename' argument".to_string()),
        };

        // Path-traversal guard
        let resolved = self.output_dir.join(&filename);
        if !resolved.starts_with(&self.output_dir) {
            return Ok(format!(
                "error: path traversal rejected for filename '{}'",
                filename
            ));
        }

        match fs::read_to_string(&resolved) {
            Ok(content) => {
                eprintln!("reading: {}", filename);
                Ok(content)
            }
            Err(e) => Ok(format!("error: could not read '{}': {}", filename, e)),
        }
    }
}
