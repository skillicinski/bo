use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::domain::index::IndexEntry;
use crate::engine::agent::{AgentError, Tool};

/// Lists all leaves in the collection. Read-only, reusable by any agent.
pub struct ListIndexTool {
    leaves: Arc<Mutex<Vec<IndexEntry>>>,
}

impl ListIndexTool {
    pub fn new(leaves: Arc<Mutex<Vec<IndexEntry>>>) -> Self {
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
        let leaves = self.leaves.lock().unwrap().clone();

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
