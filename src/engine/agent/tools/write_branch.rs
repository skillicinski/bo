use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::domain::{branch, slug};
use crate::engine::agent::{AgentError, Tool};

/// Result metadata from a single branch write.
#[derive(Debug, Clone)]
pub struct BranchResult {
    pub slug: String,
    pub title: String,
    pub leaf_count: usize,
}

/// Writes a branch (concept) file. Validates leaf filenames against a known set.
pub struct WriteBranchTool {
    branches_dir: PathBuf,
    run_timestamp: String,
    valid_filenames: Arc<HashSet<String>>,
    results: Arc<Mutex<Vec<BranchResult>>>,
}

impl WriteBranchTool {
    pub fn new(
        branches_dir: PathBuf,
        run_timestamp: String,
        valid_filenames: Arc<HashSet<String>>,
        results: Arc<Mutex<Vec<BranchResult>>>,
    ) -> Self {
        Self {
            branches_dir,
            run_timestamp,
            valid_filenames,
            results,
        }
    }
}

#[async_trait]
impl Tool for WriteBranchTool {
    fn name(&self) -> &'static str {
        "write_branch"
    }

    fn description(&self) -> &'static str {
        "Write a branch (concept) file. Call this for each recurring concept you \
         identify across the leaves. The body should be a markdown description of \
         the concept as it appears across the collection, beginning with a heading \
         matching the title. The leaves array should contain only filenames returned \
         by list_index."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Human-readable concept name (e.g. 'Rust Ownership')"
                },
                "body": {
                    "type": "string",
                    "description": "Markdown body: description of the concept across the collection"
                },
                "leaves": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Filenames (with .md) of leaves this concept appears in"
                }
            },
            "required": ["title", "body", "leaves"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value) -> Result<String, AgentError> {
        let title = match args.get("title").and_then(|v| v.as_str()) {
            Some(t) => t.to_string(),
            None => return Ok("error: missing 'title' argument".to_string()),
        };
        let body = match args.get("body").and_then(|v| v.as_str()) {
            Some(b) => b.to_string(),
            None => return Ok("error: missing 'body' argument".to_string()),
        };
        let raw_leaves: Vec<String> = args
            .get("leaves")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();

        // Validate leaves — filter out invented filenames
        let mut warnings: Vec<String> = Vec::new();
        let valid_leaves: Vec<String> = raw_leaves
            .into_iter()
            .filter(|f| {
                if self.valid_filenames.contains(f) {
                    true
                } else {
                    warnings.push(format!("unknown leaf '{}' skipped", f));
                    false
                }
            })
            .collect();

        let branch_slug = slug::slugify(&title, "");
        let branch_path = self.branches_dir.join(format!("{}.md", branch_slug));

        // Preserve compiled_at if this branch already exists
        let compiled_at =
            branch::read_compiled_at(&branch_path).unwrap_or_else(|| self.run_timestamp.clone());

        if let Err(e) = branch::write(
            &branch_path,
            &title,
            &body,
            &valid_leaves,
            &compiled_at,
            &self.run_timestamp,
        ) {
            return Ok(format!(
                "error: could not write branch '{}': {}",
                branch_slug, e
            ));
        }

        self.results.lock().unwrap().push(BranchResult {
            slug: branch_slug.clone(),
            title: title.clone(),
            leaf_count: valid_leaves.len(),
        });

        eprintln!("writing branch: {}", branch_slug);

        let mut result = format!("written: {}", branch_slug);
        if !warnings.is_empty() {
            result.push_str(&format!(" (warnings: {})", warnings.join("; ")));
        }
        Ok(result)
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashSet;
    use std::fs;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;

    use crate::domain::frontmatter;
    use crate::engine::agent::Tool;

    fn make_valid_filenames() -> Arc<HashSet<String>> {
        Arc::new(
            ["leaf-a.md", "leaf-b.md"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        )
    }

    #[tokio::test]
    async fn write_branch_creates_file_and_records_result() {
        let dir = TempDir::new().unwrap();
        let results: Arc<Mutex<Vec<BranchResult>>> = Arc::new(Mutex::new(Vec::new()));
        let tool = WriteBranchTool::new(
            dir.path().join("branches"),
            "2025-06-01T12:00:00Z".to_string(),
            make_valid_filenames(),
            Arc::clone(&results),
        );

        let result = tool
            .execute(json!({
                "title": "Test Concept",
                "body": "# Test Concept\n\nDescription.\n",
                "leaves": ["leaf-a.md", "leaf-b.md"]
            }))
            .await
            .unwrap();

        assert!(result.starts_with("written:"), "got: {}", result);

        let branch_path = dir.path().join("branches").join("test-concept.md");
        assert!(branch_path.exists());

        let r = results.lock().unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].slug, "test-concept");
        assert_eq!(r[0].leaf_count, 2);
    }

    #[tokio::test]
    async fn write_branch_first_write_compiled_at_equals_updated_at() {
        let dir = TempDir::new().unwrap();
        let results = Arc::new(Mutex::new(Vec::new()));
        let tool = WriteBranchTool::new(
            dir.path().join("branches"),
            "2025-06-01T12:00:00Z".to_string(),
            make_valid_filenames(),
            results,
        );

        tool.execute(json!({"title": "Concept", "body": "body", "leaves": []}))
            .await
            .unwrap();

        let path = dir.path().join("branches").join("concept.md");
        let content = fs::read_to_string(&path).unwrap();
        let (m, _) = frontmatter::parse(&content).unwrap();
        assert_eq!(
            m.get("compiled_at").and_then(|v| v.as_str()),
            m.get("updated_at").and_then(|v| v.as_str())
        );
    }

    #[tokio::test]
    async fn write_branch_second_write_preserves_compiled_at() {
        let dir = TempDir::new().unwrap();
        let branches_dir = dir.path().join("branches");

        // First write
        {
            let results = Arc::new(Mutex::new(Vec::new()));
            let tool = WriteBranchTool::new(
                branches_dir.clone(),
                "2025-06-01T12:00:00Z".to_string(),
                make_valid_filenames(),
                results,
            );
            tool.execute(json!({"title": "Concept", "body": "v1", "leaves": []}))
                .await
                .unwrap();
        }

        // Second write with different timestamp
        {
            let results = Arc::new(Mutex::new(Vec::new()));
            let tool = WriteBranchTool::new(
                branches_dir,
                "2025-12-01T10:00:00Z".to_string(),
                make_valid_filenames(),
                results,
            );
            tool.execute(json!({"title": "Concept", "body": "v2", "leaves": []}))
                .await
                .unwrap();
        }

        let path = dir.path().join("branches").join("concept.md");
        let content = fs::read_to_string(&path).unwrap();
        let (m, _) = frontmatter::parse(&content).unwrap();
        assert_eq!(
            m.get("compiled_at").and_then(|v| v.as_str()),
            Some("2025-06-01T12:00:00Z")
        );
        assert_eq!(
            m.get("updated_at").and_then(|v| v.as_str()),
            Some("2025-12-01T10:00:00Z")
        );
    }

    #[tokio::test]
    async fn write_branch_filters_invented_leaf_names() {
        let dir = TempDir::new().unwrap();
        let results: Arc<Mutex<Vec<BranchResult>>> = Arc::new(Mutex::new(Vec::new()));
        let tool = WriteBranchTool::new(
            dir.path().join("branches"),
            "2025-06-01T12:00:00Z".to_string(),
            make_valid_filenames(),
            Arc::clone(&results),
        );

        let result = tool
            .execute(json!({
                "title": "Concept",
                "body": "body",
                "leaves": ["leaf-a.md", "invented-nonexistent.md"]
            }))
            .await
            .unwrap();

        assert!(result.contains("written:"));
        assert!(result.contains("invented-nonexistent.md"));

        let path = dir.path().join("branches").join("concept.md");
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("leaf-a.md"));
        assert!(!content.contains("invented-nonexistent.md"));
    }
}
