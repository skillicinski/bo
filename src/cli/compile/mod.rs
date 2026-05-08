// bo compile — the compile command.
//
// This module owns:
//   - cmd_compile: entry point (setup + run phases)
//   - CompileSummary / print_summary: formatted stdout output
//   - COMPILE_SYSTEM_PROMPT: the system prompt for the compile agent
//
// Tools are defined in engine/agent/tools/ and assembled here.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use chrono::Utc;

use crate::domain::index;
use crate::domain::index::IndexEntry;
use crate::domain::leaf;
use crate::domain::tree::Tree;
use crate::engine::agent::tools::{
    BranchResult, ListIndexTool, ReadLeafTool, UpdateLeafFrontmatterTool, WriteBranchTool,
};
use crate::engine::agent::{AgentConfig, OpenAiProvider, Tool};
use crate::engine::config::Config;

// ── summary types ─────────────────────────────────────────────────────────────

pub struct CompileSummary {
    pub branches: Vec<BranchResult>,
    pub leaves_updated: usize,
    pub leaves_skipped: Vec<String>,
}

// ── system prompt ─────────────────────────────────────────────────────────────

const COMPILE_SYSTEM_PROMPT: &str = "\
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

    let valid_filenames: Arc<HashSet<String>> =
        Arc::new(valid_leaves.iter().map(|e| e.file.clone()).collect());

    let agent_config = AgentConfig {
        api_key,
        model: cfg.effective_compile_model().to_string(),
    };

    // ── shared result sinks ──────────────────────────────────────────────────
    let branches_written: Arc<Mutex<Vec<BranchResult>>> = Arc::new(Mutex::new(Vec::new()));
    let leaves_updated: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));

    // ── build tools ──────────────────────────────────────────────────────────
    let tools: Vec<Box<dyn Tool>> = vec![
        Box::new(ListIndexTool::new(Arc::new(Mutex::new(valid_leaves)))),
        Box::new(ReadLeafTool::new(cfg.tree.output_dir.clone())),
        Box::new(WriteBranchTool::new(
            branches_dir,
            run_timestamp.clone(),
            Arc::clone(&valid_filenames),
            Arc::clone(&branches_written),
        )),
        Box::new(UpdateLeafFrontmatterTool::new(
            cfg.tree.output_dir.clone(),
            run_timestamp,
            Arc::clone(&leaves_updated),
        )),
    ];

    // ── run phase ─────────────────────────────────────────────────────────────
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create async runtime: {}", e))?;

    rt.block_on(async {
        let provider = OpenAiProvider::new(&agent_config.api_key);
        let initial_message = format!(
            "Please compile my knowledge base. There are {} leaves in the collection.",
            n_valid
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
    let summary = CompileSummary {
        branches: match Arc::try_unwrap(branches_written) {
            Ok(mutex) => mutex.into_inner().unwrap_or_default(),
            Err(arc) => arc.lock().unwrap().clone(),
        },
        leaves_updated: match Arc::try_unwrap(leaves_updated) {
            Ok(mutex) => mutex.into_inner().unwrap_or_default().len(),
            Err(arc) => arc.lock().unwrap().len(),
        },
        leaves_skipped: skipped_leaves,
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

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    use serde_json::{json, Value};

    use crate::domain::frontmatter;
    use crate::engine::agent::Tool;
    use tempfile::TempDir;

    fn make_valid_filenames() -> Arc<HashSet<String>> {
        Arc::new(
            ["leaf-a.md", "leaf-b.md"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
        )
    }

    fn make_leaves() -> Vec<IndexEntry> {
        vec![
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
        ]
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
        let leaves = Arc::new(Mutex::new(make_leaves()));
        let tool = ListIndexTool::new(leaves);
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
        let tool = ReadLeafTool::new(dir.path().to_path_buf());
        let result = tool
            .execute(json!({"filename": "leaf-a.md"}))
            .await
            .unwrap();
        assert!(result.contains("Leaf A"));
    }

    #[tokio::test]
    async fn read_leaf_path_traversal_returns_error_string() {
        let dir = TempDir::new().unwrap();
        let tool = ReadLeafTool::new(dir.path().to_path_buf());
        let result = tool
            .execute(json!({"filename": "../../../etc/passwd"}))
            .await
            .unwrap();
        assert!(result.starts_with("error:"));
    }

    #[tokio::test]
    async fn read_leaf_missing_file_returns_error_string() {
        let dir = TempDir::new().unwrap();
        let tool = ReadLeafTool::new(dir.path().to_path_buf());
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

    // ── UpdateLeafFrontmatterTool ─────────────────────────────────────────────

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

    // ── guard-clause tests ────────────────────────────────────────────────────

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
