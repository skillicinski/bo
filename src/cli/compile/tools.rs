use std::collections::HashSet;
use std::fs;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::domain::{branch, frontmatter, slug};
use crate::engine::agent::{AgentError, Tool};

use super::{BranchResult, CompileContext};

// ── WriteBranchTool ───────────────────────────────────────────────────────────

pub struct WriteBranchTool {
    ctx: Arc<Mutex<CompileContext>>,
}

impl WriteBranchTool {
    pub fn new(ctx: Arc<Mutex<CompileContext>>) -> Self {
        Self { ctx }
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

        let (branches_dir, run_ts, valid_filenames) = {
            let ctx = self.ctx.lock().unwrap();
            let filenames: HashSet<String> =
                ctx.valid_leaves.iter().map(|e| e.file.clone()).collect();
            (
                ctx.branches_dir.clone(),
                ctx.run_timestamp.clone(),
                filenames,
            )
        };

        // Validate leaves — filter out invented filenames
        let mut warnings: Vec<String> = Vec::new();
        let valid_leaves: Vec<String> = raw_leaves
            .into_iter()
            .filter(|f| {
                if valid_filenames.contains(f) {
                    true
                } else {
                    warnings.push(format!("unknown leaf '{}' skipped", f));
                    false
                }
            })
            .collect();

        let branch_slug = slug::slugify(&title, "");
        let branch_path = branches_dir.join(format!("{}.md", branch_slug));

        // Preserve compiled_at if this branch already exists
        let compiled_at = branch::read_compiled_at(&branch_path).unwrap_or_else(|| run_ts.clone());

        if let Err(e) = branch::write(
            &branch_path,
            &title,
            &body,
            &valid_leaves,
            &compiled_at,
            &run_ts,
        ) {
            return Ok(format!(
                "error: could not write branch '{}': {}",
                branch_slug, e
            ));
        }

        {
            let mut ctx = self.ctx.lock().unwrap();
            ctx.branches_written.push(BranchResult {
                slug: branch_slug.clone(),
                title: title.clone(),
                leaf_count: valid_leaves.len(),
            });
        }

        eprintln!("writing branch: {}", branch_slug);

        let mut result = format!("written: {}", branch_slug);
        if !warnings.is_empty() {
            result.push_str(&format!(" (warnings: {})", warnings.join("; ")));
        }
        Ok(result)
    }
}

// ── UpdateLeafFrontmatterTool ─────────────────────────────────────────────────

pub struct UpdateLeafFrontmatterTool {
    ctx: Arc<Mutex<CompileContext>>,
}

impl UpdateLeafFrontmatterTool {
    pub fn new(ctx: Arc<Mutex<CompileContext>>) -> Self {
        Self { ctx }
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

        let (output_dir, run_ts) = {
            let ctx = self.ctx.lock().unwrap();
            (ctx.output_dir.clone(), ctx.run_timestamp.clone())
        };

        // Path-traversal guard
        let resolved = output_dir.join(&filename);
        if !resolved.starts_with(&output_dir) {
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
            &[("updated_at", run_ts.as_str())],
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

        {
            let mut ctx = self.ctx.lock().unwrap();
            ctx.leaves_updated.push(filename.clone());
        }

        eprintln!("updating: {}", filename);
        Ok(format!("updated: {}", filename))
    }
}

// ── system prompt ─────────────────────────────────────────────────────────────

pub const COMPILE_SYSTEM_PROMPT: &str = "\
You are a knowledge compilation agent for a personal document collection managed by bo.

Your task is to identify recurring concepts and themes that appear across multiple \
documents, then produce one branch file per concept and backlink every document to \
the concepts it belongs to.

## Steps

1. Call `list_index` once to discover all available documents.
2. Call `read_leaf` for each document to understand its content. You do not need to \
   re-read a document you have already read.
3. After reading, identify recurring concepts — themes, topics, or ideas that appear \
   in at least two documents. A concept must appear in at least two documents to merit \
   a branch. Prefer specific, recurring themes over broad catch-all categories.
4. For each concept, call `write_branch` with a title, a markdown body describing the \
   concept as it appears across the collection, and the list of leaves it appears in. \
   Only use filenames returned by `list_index`.
5. After writing ALL branches, call `update_leaf_frontmatter` for EVERY document — \
   including documents that belong to no branches (pass `branches: []` for those). \
   This step is mandatory for every document.
6. When all writes are complete, respond with a plain-text summary of what you produced.

## Quality guidance

- A concept must appear in at least two documents.
- Prefer specific themes over broad categories.
- Each branch body should synthesise how the concept appears across the collection, \
  not just list documents.
- Do not invent leaf filenames; only use filenames from `list_index`.
";
