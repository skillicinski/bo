// bo compile — the compile command, context, tools, and summary output.
//
// This module owns:
//   - CompileContext: state shared by all four tools during an agent run
//   - The four Tool implementations: ListIndex, ReadLeaf, WriteBranch,
//     UpdateLeafFrontmatter
//   - cmd_compile: entry point (setup + run phases)
//   - print_summary: formatted stdout output
//
// All four tools hold Arc<Mutex<CompileContext>>.  They follow the
// lock/copy/unlock pattern: lock briefly to copy needed data, drop the guard
// before any file I/O, re-lock briefly to record results.
// No MutexGuard is held across an .await point.

use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use serde_json::{json, Value};

use crate::domain::index::IndexEntry;
use crate::domain::{branch, frontmatter, slug};
use crate::engine::agent::{AgentError, Tool};

// ── context ───────────────────────────────────────────────────────────────────

pub struct CompileContext {
    pub output_dir: PathBuf,
    pub branches_dir: PathBuf,
    pub run_timestamp: String,

    // Inputs
    pub valid_leaves: Vec<IndexEntry>,
    pub skipped_leaves: Vec<String>,

    // Outputs accumulated by tools
    pub branches_written: Vec<BranchResult>,
    pub leaves_updated: Vec<String>,
}

impl CompileContext {
    pub fn into_summary(self) -> CompileSummary {
        CompileSummary {
            branches: self.branches_written,
            leaves_updated: self.leaves_updated.len(),
            leaves_skipped: self.skipped_leaves,
        }
    }
}

pub struct BranchResult {
    pub slug: String,
    pub title: String,
    pub leaf_count: usize,
}

pub struct CompileSummary {
    pub branches: Vec<BranchResult>,
    pub leaves_updated: usize,
    pub leaves_skipped: Vec<String>,
}

// ── ListIndexTool ─────────────────────────────────────────────────────────────

pub struct ListIndexTool {
    ctx: Arc<Mutex<CompileContext>>,
}

impl ListIndexTool {
    pub fn new(ctx: Arc<Mutex<CompileContext>>) -> Self {
        Self { ctx }
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
        let leaves = {
            let ctx = self.ctx.lock().unwrap();
            ctx.valid_leaves.clone()
        };

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

// ── ReadLeafTool ──────────────────────────────────────────────────────────────

pub struct ReadLeafTool {
    ctx: Arc<Mutex<CompileContext>>,
}

impl ReadLeafTool {
    pub fn new(ctx: Arc<Mutex<CompileContext>>) -> Self {
        Self { ctx }
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

        let output_dir = {
            let ctx = self.ctx.lock().unwrap();
            ctx.output_dir.clone()
        };

        // Path-traversal guard
        let resolved = output_dir.join(&filename);
        if !resolved.starts_with(&output_dir) {
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

// ── cmd_compile ───────────────────────────────────────────────────────────────

use crate::domain::index;
use crate::domain::leaf;
use crate::domain::tree::Tree;
use crate::engine::agent::{AgentConfig, OpenAiProvider};
use crate::engine::config::Config;

pub fn cmd_compile(cfg: &Config) -> Result<(), String> {
    // ── read index first (leaf count guard fires before API key check) ──────
    let index_path = cfg.tree.output_dir.join("index.jsonl");
    let all_entries =
        index::read_index(&index_path).map_err(|e| format!("failed to read index: {}", e))?;

    match all_entries.len() {
        0 => {
            println!("bo is empty!");
            return Ok(());
        }
        1 => {
            println!("bo only has 1 leaf!");
            return Ok(());
        }
        _ => {}
    }

    // ── check OPENAI_API_KEY ─────────────────────────────────────────────────
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        "OPENAI_API_KEY is not set — bo compile requires an OpenAI API key".to_string()
    })?;

    // ── validate leaves ───────────────────────────────────────────────────────
    let run_timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let mut valid_leaves: Vec<IndexEntry> = Vec::new();
    let mut skipped_leaves: Vec<String> = Vec::new();

    for entry in &all_entries {
        let leaf_path = cfg.tree.output_dir.join(&entry.file);
        match leaf::read_frontmatter(&leaf_path) {
            Ok(_) => valid_leaves.push(entry.clone()),
            Err(_) => skipped_leaves.push(entry.file.clone()),
        }
    }

    if valid_leaves.is_empty() {
        return Err(format!(
            "all {} leaves have unparseable frontmatter or are missing — nothing to compile",
            skipped_leaves.len()
        ));
    }

    let branches_dir = Tree::from_config(&cfg.tree).branches_dir();
    let n_valid = valid_leaves.len();

    // ── build context and agent config ───────────────────────────────────────
    let ctx = Arc::new(Mutex::new(CompileContext {
        output_dir: cfg.tree.output_dir.clone(),
        branches_dir,
        run_timestamp,
        valid_leaves,
        skipped_leaves,
        branches_written: Vec::new(),
        leaves_updated: Vec::new(),
    }));

    let agent_config = AgentConfig {
        api_key,
        model: cfg.effective_compile_model().to_string(),
    };

    // ── run phase ─────────────────────────────────────────────────────────────
    compile_run(ctx, agent_config, n_valid)
}

fn compile_run(
    ctx: Arc<Mutex<CompileContext>>,
    agent_config: AgentConfig,
    n_leaves: usize,
) -> Result<(), String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create async runtime: {}", e))?;

    rt.block_on(async {
        let tools: Vec<Box<dyn crate::engine::agent::Tool>> = vec![
            Box::new(ListIndexTool::new(Arc::clone(&ctx))),
            Box::new(ReadLeafTool::new(Arc::clone(&ctx))),
            Box::new(WriteBranchTool::new(Arc::clone(&ctx))),
            Box::new(UpdateLeafFrontmatterTool::new(Arc::clone(&ctx))),
        ];

        let provider = OpenAiProvider::new(&agent_config.api_key);
        let initial_message = format!(
            "Please compile my knowledge base. There are {} leaves in the collection.",
            n_leaves
        );

        let result = crate::engine::agent::run(
            &provider,
            &tools,
            &agent_config,
            COMPILE_SYSTEM_PROMPT,
            &initial_message,
            50,
        )
        .await;

        match result {
            Ok(()) => {}
            Err(crate::engine::agent::AgentError::MaxSteps(n)) => {
                eprintln!(
                    "warning: agent hit step limit ({} steps) — results may be incomplete",
                    n
                );
            }
            Err(e) => return Err(e.to_string()),
        }

        Ok(())
    })?;

    // ── extract summary ───────────────────────────────────────────────────────
    let summary = {
        // Try to unwrap the Arc; fall back to locking if other references remain
        match Arc::try_unwrap(ctx) {
            Ok(mutex) => mutex.into_inner().unwrap().into_summary(),
            Err(arc) => arc.lock().unwrap().clone_summary(),
        }
    };

    print_summary(&summary);
    Ok(())
}

// ── print_summary ─────────────────────────────────────────────────────────────

pub fn print_summary(summary: &CompileSummary) {
    if summary.branches.is_empty() {
        println!("compiled: no branches found");
    } else {
        println!(
            "compiled: {} {} across {} leaves",
            summary.branches.len(),
            if summary.branches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            summary.leaves_updated
        );
        for b in &summary.branches {
            println!(
                "  ✓ {} ({} {})",
                b.slug,
                b.leaf_count,
                if b.leaf_count == 1 { "leaf" } else { "leaves" }
            );
        }
    }

    if !summary.leaves_skipped.is_empty() {
        println!();
        println!(
            "  ⚠ skipped {} {} (unparseable frontmatter):",
            summary.leaves_skipped.len(),
            if summary.leaves_skipped.len() == 1 {
                "leaf"
            } else {
                "leaves"
            }
        );
        for f in &summary.leaves_skipped {
            println!("    - {}", f);
        }
    }
}

// ── helper for Arc fallback ───────────────────────────────────────────────────

impl CompileContext {
    fn clone_summary(&self) -> CompileSummary {
        CompileSummary {
            branches: self
                .branches_written
                .iter()
                .map(|b| BranchResult {
                    slug: b.slug.clone(),
                    title: b.title.clone(),
                    leaf_count: b.leaf_count,
                })
                .collect(),
            leaves_updated: self.leaves_updated.len(),
            leaves_skipped: self.skipped_leaves.clone(),
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::index::IndexEntry;
    use tempfile::TempDir;

    fn make_ctx(dir: &TempDir) -> Arc<Mutex<CompileContext>> {
        Arc::new(Mutex::new(CompileContext {
            output_dir: dir.path().to_path_buf(),
            branches_dir: dir.path().join("branches"),
            run_timestamp: "2025-06-01T12:00:00Z".to_string(),
            valid_leaves: vec![
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
            ],
            skipped_leaves: vec![],
            branches_written: vec![],
            leaves_updated: vec![],
        }))
    }

    fn write_leaf(dir: &TempDir, filename: &str, title: &str) {
        let content = format!(
            "---\ntitle: {}\nurl: https://example.com\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# {}\n\nBody.\n",
            title, title
        );
        fs::write(dir.path().join(filename), content).unwrap();
    }

    // ── ListIndexTool ─────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_index_returns_valid_json() {
        let dir = TempDir::new().unwrap();
        let ctx = make_ctx(&dir);
        let tool = ListIndexTool::new(ctx);
        let result = tool.execute(json!({})).await.unwrap();
        let parsed: Vec<Value> = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0]["file"].as_str(), Some("leaf-a.md"));
        assert_eq!(parsed[1]["file"].as_str(), Some("leaf-b.md"));
    }

    // ── ReadLeafTool ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn read_leaf_returns_content() {
        let dir = TempDir::new().unwrap();
        write_leaf(&dir, "leaf-a.md", "Leaf A");
        let ctx = make_ctx(&dir);
        let tool = ReadLeafTool::new(ctx);
        let result = tool
            .execute(json!({"filename": "leaf-a.md"}))
            .await
            .unwrap();
        assert!(result.contains("Leaf A"));
    }

    #[tokio::test]
    async fn read_leaf_path_traversal_returns_error_string() {
        let dir = TempDir::new().unwrap();
        let ctx = make_ctx(&dir);
        let tool = ReadLeafTool::new(ctx);
        let result = tool
            .execute(json!({"filename": "../../../etc/passwd"}))
            .await
            .unwrap();
        assert!(result.starts_with("error:"));
    }

    #[tokio::test]
    async fn read_leaf_missing_file_returns_error_string() {
        let dir = TempDir::new().unwrap();
        let ctx = make_ctx(&dir);
        let tool = ReadLeafTool::new(ctx);
        let result = tool
            .execute(json!({"filename": "nonexistent.md"}))
            .await
            .unwrap();
        assert!(result.starts_with("error:"));
    }

    // ── WriteBranchTool ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn write_branch_creates_file_and_records_result() {
        let dir = TempDir::new().unwrap();
        let ctx = make_ctx(&dir);
        let tool = WriteBranchTool::new(Arc::clone(&ctx));

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

        let ctx_guard = ctx.lock().unwrap();
        assert_eq!(ctx_guard.branches_written.len(), 1);
        assert_eq!(ctx_guard.branches_written[0].slug, "test-concept");
        assert_eq!(ctx_guard.branches_written[0].leaf_count, 2);
    }

    #[tokio::test]
    async fn write_branch_first_write_compiled_at_equals_updated_at() {
        let dir = TempDir::new().unwrap();
        let ctx = make_ctx(&dir);
        let tool = WriteBranchTool::new(Arc::clone(&ctx));

        tool.execute(json!({
            "title": "Concept",
            "body": "body",
            "leaves": []
        }))
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

        // First write
        {
            let ctx = make_ctx(&dir);
            let tool = WriteBranchTool::new(ctx);
            tool.execute(json!({"title": "Concept", "body": "v1", "leaves": []}))
                .await
                .unwrap();
        }

        // Second write with different run_timestamp
        let ctx2 = Arc::new(Mutex::new(CompileContext {
            output_dir: dir.path().to_path_buf(),
            branches_dir: dir.path().join("branches"),
            run_timestamp: "2025-12-01T10:00:00Z".to_string(),
            valid_leaves: vec![],
            skipped_leaves: vec![],
            branches_written: vec![],
            leaves_updated: vec![],
        }));
        let tool2 = WriteBranchTool::new(ctx2);
        tool2
            .execute(json!({"title": "Concept", "body": "v2", "leaves": []}))
            .await
            .unwrap();

        let path = dir.path().join("branches").join("concept.md");
        let content = fs::read_to_string(&path).unwrap();
        let (m, _) = frontmatter::parse(&content).unwrap();
        // compiled_at is from first write
        assert_eq!(
            m.get("compiled_at").and_then(|v| v.as_str()),
            Some("2025-06-01T12:00:00Z")
        );
        // updated_at is from second write
        assert_eq!(
            m.get("updated_at").and_then(|v| v.as_str()),
            Some("2025-12-01T10:00:00Z")
        );
    }

    #[tokio::test]
    async fn write_branch_filters_invented_leaf_names() {
        let dir = TempDir::new().unwrap();
        let ctx = make_ctx(&dir);
        let tool = WriteBranchTool::new(Arc::clone(&ctx));

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

        // Only valid leaf should be in the branch file
        let path = dir.path().join("branches").join("concept.md");
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("leaf-a.md"));
        assert!(!content.contains("invented-nonexistent.md"));
    }

    // ── UpdateLeafFrontmatterTool ─────────────────────────────────────────────

    #[tokio::test]
    async fn update_leaf_frontmatter_adds_branches_field() {
        let dir = TempDir::new().unwrap();
        write_leaf(&dir, "leaf-a.md", "Leaf A");
        let ctx = make_ctx(&dir);
        let tool = UpdateLeafFrontmatterTool::new(Arc::clone(&ctx));

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
        let ctx = make_ctx(&dir);
        let tool = UpdateLeafFrontmatterTool::new(Arc::clone(&ctx));

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

        let ctx = make_ctx(&dir);
        let tool = UpdateLeafFrontmatterTool::new(ctx);
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
        let ctx = make_ctx(&dir);
        let tool = UpdateLeafFrontmatterTool::new(ctx);
        let result = tool
            .execute(json!({"filename": "../etc/passwd", "branches": []}))
            .await
            .unwrap();
        assert!(result.starts_with("error:"));
    }

    // ── guard-clause tests (moved from tests/integration_compile.rs) ───────

    fn make_test_config(output_dir: &std::path::Path) -> Config {
        Config {
            tree: crate::domain::tree::TreeConfig {
                output_dir: output_dir.to_path_buf(),
                name: None,
                created_at: None,
            },
            compile_model: Some("gpt-4o-mini".to_string()),
        }
    }

    #[test]
    fn compile_exits_cleanly_on_empty_collection() {
        let dir = TempDir::new().unwrap();
        let cfg = make_test_config(dir.path());
        std::env::remove_var("OPENAI_API_KEY");
        fs::write(dir.path().join("index.jsonl"), "").unwrap();
        let result = cmd_compile(&cfg);
        assert!(result.is_ok());
    }

    #[test]
    fn compile_exits_cleanly_on_single_leaf() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("index.jsonl"),
            r#"{"file":"only.md","title":"Only","url":"https://example.com"}"#,
        )
        .unwrap();
        std::env::remove_var("OPENAI_API_KEY");
        let cfg = make_test_config(dir.path());
        let result = cmd_compile(&cfg);
        assert!(result.is_ok());
    }

    #[test]
    fn compile_errors_without_api_key() {
        let dir = TempDir::new().unwrap();
        let index_path = dir.path().join("index.jsonl");
        fs::write(
            &index_path,
            r#"{"file":"a.md","title":"A","url":"https://example.com/a"}
{"file":"b.md","title":"B","url":"https://example.com/b"}"#,
        )
        .unwrap();
        std::env::remove_var("OPENAI_API_KEY");
        let cfg = make_test_config(dir.path());
        let result = cmd_compile(&cfg);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(
            msg.contains("OPENAI_API_KEY"),
            "error message should mention OPENAI_API_KEY, got: {}",
            msg
        );
    }
}
