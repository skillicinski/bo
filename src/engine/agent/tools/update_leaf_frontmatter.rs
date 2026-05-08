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

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    use crate::engine::agent::Tool;

    fn write_leaf(dir: &TempDir, filename: &str, title: &str) {
        let content = format!(
            "---\ntitle: {}\nurl: https://example.com\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# {}\n\nBody.\n",
            title, title
        );
        fs::write(dir.path().join(filename), content).unwrap();
    }

    #[tokio::test]
    async fn update_leaf_frontmatter_adds_branches_field() {
        let dir = TempDir::new().unwrap();
        write_leaf(&dir, "leaf-a.md", "Leaf A");
        let results = Arc::new(Mutex::new(Vec::new()));
        let tool = UpdateLeafFrontmatterTool::new(
            dir.path().to_path_buf(),
            "2025-06-01T12:00:00Z".to_string(),
            results,
        );

        let result = tool
            .execute(json!({
                "filename": "leaf-a.md",
                "branches": ["concept-a", "concept-b"]
            }))
            .await
            .unwrap();

        assert_eq!(result, "updated: leaf-a.md");

        let content = fs::read_to_string(dir.path().join("leaf-a.md")).unwrap();
        assert!(content.contains("branches:"));
        assert!(content.contains("- concept-a"));
        assert!(content.contains("- concept-b"));
        assert!(content.contains("updated_at: 2025-06-01T12:00:00Z"));
    }

    #[tokio::test]
    async fn update_leaf_frontmatter_empty_branches_writes_inline_empty() {
        let dir = TempDir::new().unwrap();
        write_leaf(&dir, "leaf-a.md", "Leaf A");
        let results = Arc::new(Mutex::new(Vec::new()));
        let tool = UpdateLeafFrontmatterTool::new(
            dir.path().to_path_buf(),
            "2025-06-01T12:00:00Z".to_string(),
            results,
        );

        tool.execute(json!({"filename": "leaf-a.md", "branches": []}))
            .await
            .unwrap();

        let content = fs::read_to_string(dir.path().join("leaf-a.md")).unwrap();
        assert!(content.contains("branches: []"));
    }

    #[tokio::test]
    async fn update_leaf_frontmatter_body_is_byte_identical() {
        let dir = TempDir::new().unwrap();
        write_leaf(&dir, "leaf-a.md", "Leaf A");
        let original = fs::read_to_string(dir.path().join("leaf-a.md")).unwrap();
        let orig_body = original.split("\n---\n\n").nth(1).unwrap().to_string();

        let results = Arc::new(Mutex::new(Vec::new()));
        let tool = UpdateLeafFrontmatterTool::new(
            dir.path().to_path_buf(),
            "2025-06-01T12:00:00Z".to_string(),
            results,
        );
        tool.execute(json!({"filename": "leaf-a.md", "branches": ["x"]}))
            .await
            .unwrap();

        let updated = fs::read_to_string(dir.path().join("leaf-a.md")).unwrap();
        let new_body = updated.split("\n---\n\n").nth(1).unwrap();
        assert_eq!(orig_body, new_body);
    }

    #[tokio::test]
    async fn update_leaf_frontmatter_path_traversal_returns_error() {
        let dir = TempDir::new().unwrap();
        let results = Arc::new(Mutex::new(Vec::new()));
        let tool = UpdateLeafFrontmatterTool::new(
            dir.path().to_path_buf(),
            "2025-06-01T12:00:00Z".to_string(),
            results,
        );
        let result = tool
            .execute(json!({"filename": "../etc/passwd", "branches": []}))
            .await
            .unwrap();
        assert!(result.starts_with("error:"));
    }
}
