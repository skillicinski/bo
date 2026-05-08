use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::domain::frontmatter;
use crate::engine::agent::{AgentError, Tool};

/// Updates a leaf's frontmatter with branch assignments and a new updated_at timestamp.
pub struct UpdateLeafFrontmatterTool {
    output_dir: PathBuf,
    run_timestamp: String,
    results: Arc<Mutex<Vec<String>>>,
}

impl UpdateLeafFrontmatterTool {
    pub fn new(
        output_dir: PathBuf,
        run_timestamp: String,
        results: Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            output_dir,
            run_timestamp,
            results,
        }
    }
}

#[async_trait]
impl Tool for UpdateLeafFrontmatterTool {
    fn name(&self) -> &'static str {
        "update_leaf_frontmatter"
    }

    fn description(&self) -> &'static str {
        "Update a leaf document's frontmatter to record which branches it belongs to. \
         Call this for EVERY leaf after writing all branches — including leaves with \
         no matching branches (pass an empty array for those). This backlinks each \
         document to the concepts it contributes to."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "filename": {
                    "type": "string",
                    "description": "The leaf filename (with .md extension)"
                },
                "branches": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Branch slugs this leaf belongs to (empty array [] if none)"
                }
            },
            "required": ["filename", "branches"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        let filename = match args.get("filename").and_then(|v| v.as_str()) {
            Some(f) => f.to_string(),
            None => return Ok("error: missing 'filename' argument".to_string()),
        };
        let branches: Vec<String> = args
            .get("branches")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        // Path-traversal guard
        let resolved = self.output_dir.join(&filename);
        if !resolved.starts_with(&self.output_dir) {
            return Ok(format!(
                "error: path traversal rejected for filename '{}'",
                filename
            ));
        }

        let content = match fs::read_to_string(&resolved) {
            Ok(c) => c,
            Err(e) => return Ok(format!("error: could not read '{}': {}", filename, e)),
        };

        let updated = match frontmatter::patch_fields(
            &content,
            &[("updated_at", self.run_timestamp.as_str())],
            &[("branches", &branches)],
        ) {
            Ok(s) => s,
            Err(e) => {
                return Ok(format!(
                    "error: could not patch frontmatter of '{}': {}",
                    filename, e
                ))
            }
        };

        if let Err(e) = fs::write(&resolved, &updated) {
            return Ok(format!("error: could not write '{}': {}", filename, e));
        }

        self.results.lock().unwrap().push(filename.clone());

        eprintln!("updating: {}", filename);
        Ok(format!("updated: {}", filename))
    }
}
