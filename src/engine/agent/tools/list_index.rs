use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::domain::index::IndexEntry;
use crate::engine::agent::{AgentError, Tool};

/// Lists all leaves in the collection. Read-only, reusable by any agent.
pub struct ListIndexTool {
    leaves: Arc<Vec<IndexEntry>>,
}

impl ListIndexTool {
    pub fn new(leaves: Arc<Vec<IndexEntry>>) -> Self {
        Self { leaves }
    }
}

#[async_trait]
impl Tool for ListIndexTool {
    fn name(&self) -> &'static str {
        "list_index"
    }

    fn description(&self) -> &'static str {
        "List all leaves (documents) in the bo collection. Returns a JSON array of \
         objects with 'file', 'title', and 'url' fields. Call this once at the start \
         to discover available documents."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    async fn execute(&self, _args: Value) -> Result<String, AgentError> {
        let leaves = &*self.leaves;

        eprintln!("reading index ({} leaves)…", leaves.len());

        let arr: Vec<Value> = leaves
            .iter()
            .map(|e| {
                json!({
                    "file": e.file,
                    "title": e.title,
                    "url": e.url,
                })
            })
            .collect();

        Ok(serde_json::to_string(&arr).unwrap_or_else(|_| "[]".to_string()))
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    use crate::engine::agent::Tool;

    #[tokio::test]
    async fn list_index_returns_valid_json() {
        let leaves = Arc::new(vec![
            IndexEntry {
                file: "leaf-a.md".to_string(),
                title: "Leaf A".to_string(),
                url: "https://example.com/a".to_string(),
            },
            IndexEntry {
                file: "leaf-b.md".to_string(),
                title: "Leaf B".to_string(),
                url: "https://example.com/b".to_string(),
            },
        ]);
        let tool = ListIndexTool::new(leaves);
        let result = tool.execute(json!({})).await.unwrap();
        let parsed: Vec<Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["file"].as_str(), Some("leaf-a.md"));
        assert_eq!(parsed[1]["file"].as_str(), Some("leaf-b.md"));
    }
}
