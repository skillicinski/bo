// bo compile — deterministic pipeline with a single structured LLM call.
//
// Pipeline: read leaves → build prompt → LLM call → parse/validate → write → summary
//
// No agent loop, no tool dispatch. The LLM receives all leaf content and returns
// a structured JSON response with identified concepts (branches) and their
// leaf associations.

use std::collections::{HashMap, HashSet};
use std::fs;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::domain::{branch, frontmatter, index, slug, tree::Tree};
use crate::engine::config::Config;
use crate::engine::llm::{FinishReason, LlmError, Message, OpenAiProvider};

// ── constants ─────────────────────────────────────────────────────────────────

const MAX_COMPLETION_TOKENS: u32 = 16384;

// ── errors ────────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum CompileError {
    /// Collection exceeds the model's context window.
    ContextOverflow,
    /// LLM output was truncated (hit max_completion_tokens).
    Truncated,
    /// Response blocked by content filter.
    ContentFilter,
    /// LLM API or network error.
    Llm(String),
    /// I/O or index error.
    Io(String),
    /// Validation error in the LLM response.
    Validation(String),
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompileError::ContextOverflow => write!(
                f,
                "collection too large for model context window — \
                 try reducing collection size or using a model with larger context"
            ),
            CompileError::Truncated => write!(
                f,
                "compile output was truncated — try reducing collection size or \
                 using a model with larger output capacity"
            ),
            CompileError::ContentFilter => write!(f, "compile was blocked by content filter"),
            CompileError::Llm(msg) => write!(f, "LLM error: {}", msg),
            CompileError::Io(msg) => write!(f, "{}", msg),
            CompileError::Validation(msg) => write!(f, "{}", msg),
        }
    }
}

const COMPILE_SYSTEM_PROMPT: &str = "\
You are a knowledge compilation engine for a personal document collection.

Your task: identify recurring concepts and themes that appear across multiple \
documents, then produce structured output describing each concept.

## Rules

- A concept MUST appear in at least two documents. Never create a branch with only one leaf. \
  If a topic only appears in a single document, do not create a branch for it — it is not a \
  cross-cutting concept.
- Prefer specific, recurring themes over broad catch-all categories.
- Each branch body should synthesise how the concept manifests across the documents — \
  draw connections, note contrasts, highlight patterns. Do not just summarise each document \
  in turn.
- The body should begin with a single markdown heading matching the title (e.g. `# Concept Name`). \
  Do not repeat the heading or nest a second heading immediately after.
- Reference documents by their filename only when making a specific point about that document's \
  contribution to the concept.
- Only use document filenames exactly as provided in the input.
- If no cross-cutting concepts span two or more documents, return an empty branches array.
";

// ── public types ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct CompileResult {
    pub status: String,
    pub reason: Option<String>,
    pub branches: Vec<BranchResult>,
    pub leaves_updated: usize,
    pub leaves_skipped: Vec<String>,
}

impl CompileResult {
    fn compiled(summary: CompileSummary) -> Self {
        Self {
            status: "compiled".to_string(),
            reason: None,
            branches: summary.branches,
            leaves_updated: summary.leaves_updated,
            leaves_skipped: summary.leaves_skipped,
        }
    }

    fn noop(reason: &str) -> Self {
        Self {
            status: "noop".to_string(),
            reason: Some(reason.to_string()),
            branches: Vec::new(),
            leaves_updated: 0,
            leaves_skipped: Vec::new(),
        }
    }
}

pub struct CompileSummary {
    pub branches: Vec<BranchResult>,
    pub leaves_updated: usize,
    pub leaves_skipped: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BranchResult {
    pub slug: String,
    pub title: String,
    pub leaf_count: usize,
}

// ── internal types ────────────────────────────────────────────────────────────

/// A leaf with its full content loaded for prompt assembly.
struct LoadedLeaf {
    filename: String,
    title: String,
    body: String,
}

/// Deserialized LLM response.
#[derive(Deserialize)]
struct CompileResponse {
    branches: Vec<RawBranch>,
}

#[derive(Deserialize)]
struct RawBranch {
    title: String,
    body: String,
    leaves: Vec<String>,
}

/// Validated compile plan ready for execution.
#[derive(Debug)]
struct CompilePlan {
    branches: Vec<ValidatedBranch>,
    /// leaf filename → list of branch slugs it belongs to
    leaf_assignments: HashMap<String, Vec<String>>,
}

#[derive(Debug)]
struct ValidatedBranch {
    slug: String,
    title: String,
    body: String,
    leaves: Vec<String>,
}

// ── cmd_compile ───────────────────────────────────────────────────────────────

pub fn cmd_compile(cfg: &Config) -> Result<(), String> {
    let result = run_compile(cfg).map_err(|e| e.to_string())?;
    print_result(&result);
    Ok(())
}

pub fn run_compile(cfg: &Config) -> Result<CompileResult, CompileError> {
    // ── read index (guard: empty/single-leaf) ────────────────────────────────
    let index_path = cfg.tree.output_dir.join("index.jsonl");
    let all_entries = index::read_index(&index_path)
        .map_err(|e| CompileError::Io(format!("failed to read index: {}", e)))?;

    match all_entries.len() {
        0 => return Ok(CompileResult::noop("empty_tree")),
        1 => return Ok(CompileResult::noop("single_leaf")),
        _ => {}
    }

    // ── check OPENAI_API_KEY ─────────────────────────────────────────────────
    let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
        CompileError::Io(
            "OPENAI_API_KEY is not set — bo compile requires an OpenAI API key".to_string(),
        )
    })?;

    // ── load valid leaves ────────────────────────────────────────────────────
    let (loaded_leaves, skipped_leaves) = read_valid_leaves(cfg, &all_entries);

    if loaded_leaves.is_empty() {
        return Err(CompileError::Io(format!(
            "all {} leaves have unparseable frontmatter or are missing — nothing to compile",
            skipped_leaves.len()
        )));
    }

    if loaded_leaves.len() < 2 {
        return Ok(CompileResult::noop("single_leaf"));
    }

    // ── build prompt and schema ──────────────────────────────────────────────
    let user_message = build_user_message(&loaded_leaves);
    let schema = compile_response_schema();

    // ── LLM call ─────────────────────────────────────────────────────────────
    let response = call_llm(
        &api_key,
        cfg.effective_compile_model(),
        &user_message,
        &schema,
    )?;

    // ── parse and validate ───────────────────────────────────────────────────
    let valid_filenames: HashSet<String> =
        loaded_leaves.iter().map(|l| l.filename.clone()).collect();
    let plan = parse_and_validate(&response, &valid_filenames)?;

    // ── execute plan ─────────────────────────────────────────────────────────
    let run_timestamp = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let summary = execute_plan(
        &plan,
        cfg,
        &valid_filenames,
        &run_timestamp,
        &skipped_leaves,
    )?;

    // ── output ───────────────────────────────────────────────────────────────
    Ok(CompileResult::compiled(summary))
}

// ── read_valid_leaves ─────────────────────────────────────────────────────────

fn read_valid_leaves(
    cfg: &Config,
    entries: &[index::IndexEntry],
) -> (Vec<LoadedLeaf>, Vec<String>) {
    let mut loaded = Vec::new();
    let mut skipped = Vec::new();

    for entry in entries {
        let leaf_path = cfg.tree.output_dir.join(&entry.file);
        match fs::read_to_string(&leaf_path) {
            Ok(content) => match frontmatter::parse(&content) {
                Ok((mapping, body)) => {
                    let title = mapping
                        .get("title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    loaded.push(LoadedLeaf {
                        filename: entry.file.clone(),
                        title,
                        body,
                    });
                }
                Err(_) => skipped.push(entry.file.clone()),
            },
            Err(_) => skipped.push(entry.file.clone()),
        }
    }

    (loaded, skipped)
}

// ── build_user_message ────────────────────────────────────────────────────────

fn build_user_message(leaves: &[LoadedLeaf]) -> String {
    let mut msg = format!(
        "Please compile my knowledge base. There are {} documents.\n\n",
        leaves.len()
    );

    for leaf in leaves {
        msg.push_str(&format!(
            "<document filename=\"{}\" title=\"{}\">\n{}\n</document>\n\n",
            leaf.filename, leaf.title, leaf.body
        ));
    }

    msg
}

// ── compile_response_schema ───────────────────────────────────────────────────

fn compile_response_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "branches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "title": {
                            "type": "string",
                            "description": "Human-readable concept name"
                        },
                        "body": {
                            "type": "string",
                            "description": "Markdown body describing the concept across the collection"
                        },
                        "leaves": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Filenames (with .md) of leaves this concept appears in"
                        }
                    },
                    "required": ["title", "body", "leaves"],
                    "additionalProperties": false
                }
            }
        },
        "required": ["branches"],
        "additionalProperties": false
    })
}

// ── call_llm ──────────────────────────────────────────────────────────────────

fn call_llm(
    api_key: &str,
    model: &str,
    user_message: &str,
    schema: &Value,
) -> Result<String, CompileError> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| CompileError::Io(format!("failed to create async runtime: {}", e)))?;

    rt.block_on(async {
        let provider = OpenAiProvider::new(api_key);
        let messages = vec![
            Message::system(COMPILE_SYSTEM_PROMPT),
            Message::user(user_message),
        ];

        use crate::engine::llm::LlmProvider;
        let response = provider
            .complete(&messages, model, MAX_COMPLETION_TOKENS, Some(schema))
            .await
            .map_err(|e| match e {
                LlmError::Api(msg) if msg.contains("maximum context length") => {
                    CompileError::ContextOverflow
                }
                other => CompileError::Llm(other.to_string()),
            })?;

        if response.finish_reason == FinishReason::Length {
            return Err(CompileError::Truncated);
        }

        if response.finish_reason == FinishReason::ContentFilter {
            return Err(CompileError::ContentFilter);
        }

        Ok(response.content)
    })
}

// ── parse_and_validate ────────────────────────────────────────────────────────

fn parse_and_validate(
    response: &str,
    valid_filenames: &HashSet<String>,
) -> Result<CompilePlan, CompileError> {
    let parsed: CompileResponse = serde_json::from_str(response)
        .map_err(|e| CompileError::Validation(format!("failed to parse LLM response: {}", e)))?;

    // Empty branches is valid — means no cross-cutting concepts found
    if parsed.branches.is_empty() {
        return Ok(CompilePlan {
            branches: Vec::new(),
            leaf_assignments: HashMap::new(),
        });
    }

    let mut validated_branches: Vec<ValidatedBranch> = Vec::new();
    let mut seen_slugs: HashSet<String> = HashSet::new();
    let mut leaf_assignments: HashMap<String, Vec<String>> = HashMap::new();

    for raw in parsed.branches {
        // Validate non-empty title and body
        if raw.title.trim().is_empty() {
            eprintln!("warning: skipping branch with empty title");
            continue;
        }
        if raw.body.trim().is_empty() {
            eprintln!("warning: skipping branch '{}' with empty body", raw.title);
            continue;
        }

        // Generate slug and check uniqueness post-slugification
        let branch_slug = slug::slugify(&raw.title, "");
        if branch_slug.is_empty() {
            eprintln!(
                "warning: skipping branch '{}' — title produces empty slug",
                raw.title
            );
            continue;
        }
        if seen_slugs.contains(&branch_slug) {
            return Err(CompileError::Validation(format!(
                "duplicate branch slug '{}' (from title '{}') — titles must be distinct",
                branch_slug, raw.title
            )));
        }
        seen_slugs.insert(branch_slug.clone());

        // Filter and deduplicate leaves
        let mut branch_leaves: Vec<String> = Vec::new();
        let mut seen_leaves: HashSet<String> = HashSet::new();
        for leaf_file in &raw.leaves {
            if !valid_filenames.contains(leaf_file) {
                eprintln!(
                    "warning: branch '{}' references unknown leaf '{}' — skipped",
                    raw.title, leaf_file
                );
                continue;
            }
            if seen_leaves.insert(leaf_file.clone()) {
                branch_leaves.push(leaf_file.clone());
            }
        }

        // Skip branches with fewer than 2 valid leaves (must be cross-cutting)
        if branch_leaves.len() < 2 {
            eprintln!(
                "warning: skipping branch '{}' — only {} leaf (must span at least 2)",
                raw.title,
                branch_leaves.len()
            );
            continue;
        }

        // Record leaf assignments
        for leaf_file in &branch_leaves {
            leaf_assignments
                .entry(leaf_file.clone())
                .or_default()
                .push(branch_slug.clone());
        }

        validated_branches.push(ValidatedBranch {
            slug: branch_slug,
            title: raw.title,
            body: raw.body,
            leaves: branch_leaves,
        });
    }

    Ok(CompilePlan {
        branches: validated_branches,
        leaf_assignments,
    })
}

// ── execute_plan ──────────────────────────────────────────────────────────────

fn execute_plan(
    plan: &CompilePlan,
    cfg: &Config,
    valid_filenames: &HashSet<String>,
    run_timestamp: &str,
    skipped_leaves: &[String],
) -> Result<CompileSummary, CompileError> {
    let tree = Tree::from_config(&cfg.tree);
    let branches_dir = tree.branches_dir();

    // ── write branches ───────────────────────────────────────────────────────
    let mut branch_results: Vec<BranchResult> = Vec::new();

    for vb in &plan.branches {
        let branch_path = branches_dir.join(format!("{}.md", vb.slug));

        // Preserve compiled_at from existing branch if it exists
        let compiled_at =
            branch::read_compiled_at(&branch_path).unwrap_or_else(|| run_timestamp.to_string());

        branch::write(
            &branch_path,
            &vb.title,
            &vb.body,
            &vb.leaves,
            &compiled_at,
            run_timestamp,
        )
        .map_err(|e| CompileError::Io(format!("failed to write branch '{}': {}", vb.slug, e)))?;

        eprintln!("writing branch: {}", vb.slug);

        branch_results.push(BranchResult {
            slug: vb.slug.clone(),
            title: vb.title.clone(),
            leaf_count: vb.leaves.len(),
        });
    }

    // ── update leaf frontmatter ──────────────────────────────────────────────
    let mut leaves_updated: usize = 0;

    for filename in valid_filenames {
        let leaf_path = cfg.tree.output_dir.join(filename);

        // Get this leaf's branch assignments (empty if not in any branch)
        let branches: Vec<String> = plan
            .leaf_assignments
            .get(filename)
            .cloned()
            .unwrap_or_default();

        let content = match fs::read_to_string(&leaf_path) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("warning: could not read '{}': {} — skipping", filename, e);
                continue;
            }
        };

        let updated = match frontmatter::patch_fields(
            &content,
            &[("updated_at", run_timestamp)],
            &[("branches", &branches)],
        ) {
            Ok(s) => s,
            Err(e) => {
                eprintln!(
                    "warning: could not patch frontmatter of '{}': {} — skipping",
                    filename, e
                );
                continue;
            }
        };

        if let Err(e) = fs::write(&leaf_path, &updated) {
            eprintln!("warning: could not write '{}': {} — skipping", filename, e);
            continue;
        }

        leaves_updated += 1;
    }

    Ok(CompileSummary {
        branches: branch_results,
        leaves_updated,
        leaves_skipped: skipped_leaves.to_vec(),
    })
}

// ── print_summary ─────────────────────────────────────────────────────────────

pub fn print_result(result: &CompileResult) {
    if result.status == "noop" {
        match result.reason.as_deref() {
            Some("empty_tree") => println!("bo is empty!"),
            Some("single_leaf") => println!("bo only has 1 leaf!"),
            _ => println!("compiled: no work to do"),
        }
        return;
    }

    print_summary_parts(
        &result.branches,
        result.leaves_updated,
        &result.leaves_skipped,
    );
}

fn print_summary_parts(
    branches: &[BranchResult],
    leaves_updated: usize,
    leaves_skipped: &[String],
) {
    if branches.is_empty() {
        println!("compiled: no branches found");
    } else {
        println!(
            "compiled: {} {} across {} leaves",
            branches.len(),
            if branches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            leaves_updated
        );
        for b in branches {
            println!(
                "  ✓ {} ({} {})",
                b.slug,
                b.leaf_count,
                if b.leaf_count == 1 { "leaf" } else { "leaves" }
            );
        }
    }

    if !leaves_skipped.is_empty() {
        println!();
        println!(
            "  ⚠ skipped {} {} (unparseable frontmatter):",
            leaves_skipped.len(),
            if leaves_skipped.len() == 1 {
                "leaf"
            } else {
                "leaves"
            }
        );
        for f in leaves_skipped {
            println!("    - {}", f);
        }
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::fs;
    use tempfile::TempDir;

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

    // ── guard tests (ported) ──────────────────────────────────────────────────

    #[test]
    #[serial]
    fn compile_exits_cleanly_on_empty_collection() {
        let dir = TempDir::new().unwrap();
        let cfg = make_test_config(dir.path());
        std::env::remove_var("OPENAI_API_KEY");
        fs::write(dir.path().join("index.jsonl"), "").unwrap();
        let result = cmd_compile(&cfg);
        assert!(result.is_ok());
    }

    #[test]
    #[serial]
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
    #[serial]
    fn compile_errors_without_api_key() {
        let dir = TempDir::new().unwrap();
        let index_path = dir.path().join("index.jsonl");
        // Write two valid leaves so we pass the guard
        fs::write(
            &index_path,
            r#"{"file":"a.md","title":"A","url":"https://example.com/a"}
{"file":"b.md","title":"B","url":"https://example.com/b"}"#,
        )
        .unwrap();
        // Write actual leaf files with valid frontmatter
        fs::write(
            dir.path().join("a.md"),
            "---\ntitle: A\nurl: https://example.com/a\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# A\n\nBody A.\n",
        ).unwrap();
        fs::write(
            dir.path().join("b.md"),
            "---\ntitle: B\nurl: https://example.com/b\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# B\n\nBody B.\n",
        ).unwrap();
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

    // ── parse_and_validate tests ──────────────────────────────────────────────

    fn sample_valid_filenames() -> HashSet<String> {
        ["leaf-a.md", "leaf-b.md", "leaf-c.md"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn parse_valid_response() {
        let json = serde_json::json!({
            "branches": [
                {
                    "title": "Test Concept",
                    "body": "# Test Concept\n\nDescription.",
                    "leaves": ["leaf-a.md", "leaf-b.md"]
                }
            ]
        })
        .to_string();
        let json = &json;

        let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
        assert_eq!(plan.branches.len(), 1);
        assert_eq!(plan.branches[0].slug, "test-concept");
        assert_eq!(plan.branches[0].leaves.len(), 2);
        assert_eq!(
            plan.leaf_assignments.get("leaf-a.md").unwrap(),
            &vec!["test-concept".to_string()]
        );
    }

    #[test]
    fn parse_empty_branches_is_valid() {
        let json = r#"{"branches": []}"#;
        let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
        assert!(plan.branches.is_empty());
        assert!(plan.leaf_assignments.is_empty());
    }

    #[test]
    fn parse_filters_unknown_leaves() {
        let json = serde_json::json!({
            "branches": [
                {
                    "title": "Concept",
                    "body": "# Concept\n\nBody.",
                    "leaves": ["leaf-a.md", "leaf-b.md", "invented.md"]
                }
            ]
        })
        .to_string();
        let json = &json;

        let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
        assert_eq!(plan.branches[0].leaves, vec!["leaf-a.md", "leaf-b.md"]);
    }

    #[test]
    fn parse_deduplicates_leaves_within_branch() {
        let json = serde_json::json!({
            "branches": [
                {
                    "title": "Concept",
                    "body": "# Concept\n\nBody.",
                    "leaves": ["leaf-a.md", "leaf-a.md", "leaf-b.md"]
                }
            ]
        })
        .to_string();
        let json = &json;

        let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
        assert_eq!(plan.branches[0].leaves, vec!["leaf-a.md", "leaf-b.md"]);
    }

    #[test]
    fn parse_rejects_duplicate_slugs() {
        // "Rust Ownership" and "Rust: Ownership" both slugify to "rust-ownership"
        let json = serde_json::json!({
            "branches": [
                {
                    "title": "Rust Ownership",
                    "body": "# Rust Ownership\n\nBody.",
                    "leaves": ["leaf-a.md"]
                },
                {
                    "title": "Rust: Ownership",
                    "body": "# Rust: Ownership\n\nBody.",
                    "leaves": ["leaf-b.md"]
                }
            ]
        })
        .to_string();
        let json = &json;

        let result = parse_and_validate(json, &sample_valid_filenames());
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("duplicate branch slug"));
    }

    #[test]
    fn parse_skips_branch_with_all_unknown_leaves() {
        let json = serde_json::json!({
            "branches": [
                {
                    "title": "Concept",
                    "body": "# Concept\n\nBody.",
                    "leaves": ["nonexistent.md"]
                }
            ]
        })
        .to_string();
        let json = &json;

        let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
        assert!(plan.branches.is_empty());
    }

    #[test]
    fn parse_skips_branch_with_single_leaf() {
        let json = serde_json::json!({
            "branches": [
                {
                    "title": "Solo Concept",
                    "body": "# Solo Concept\n\nBody.",
                    "leaves": ["leaf-a.md"]
                }
            ]
        })
        .to_string();
        let json = &json;

        let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
        assert!(plan.branches.is_empty());
    }

    #[test]
    fn parse_skips_branch_with_empty_title() {
        let json = serde_json::json!({
            "branches": [
                {
                    "title": "",
                    "body": "# Something\n\nBody.",
                    "leaves": ["leaf-a.md"]
                }
            ]
        })
        .to_string();
        let json = &json;

        let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
        assert!(plan.branches.is_empty());
    }

    #[test]
    fn parse_skips_branch_with_empty_body() {
        let json = serde_json::json!({
            "branches": [
                {
                    "title": "Concept",
                    "body": "",
                    "leaves": ["leaf-a.md"]
                }
            ]
        })
        .to_string();
        let json = &json;

        let plan = parse_and_validate(json, &sample_valid_filenames()).unwrap();
        assert!(plan.branches.is_empty());
    }

    // ── execute_plan tests ────────────────────────────────────────────────────

    #[test]
    fn execute_plan_writes_branches_and_updates_frontmatter() {
        let dir = TempDir::new().unwrap();
        let cfg = make_test_config(dir.path());

        // Write leaf files
        fs::write(
            dir.path().join("leaf-a.md"),
            "---\ntitle: A\nurl: https://example.com/a\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# A\n\nBody.\n",
        ).unwrap();
        fs::write(
            dir.path().join("leaf-b.md"),
            "---\ntitle: B\nurl: https://example.com/b\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# B\n\nBody.\n",
        ).unwrap();

        let valid_filenames: HashSet<String> = ["leaf-a.md", "leaf-b.md"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        let mut leaf_assignments = HashMap::new();
        leaf_assignments.insert("leaf-a.md".to_string(), vec!["test-concept".to_string()]);
        leaf_assignments.insert("leaf-b.md".to_string(), vec!["test-concept".to_string()]);

        let plan = CompilePlan {
            branches: vec![ValidatedBranch {
                slug: "test-concept".to_string(),
                title: "Test Concept".to_string(),
                body: "# Test Concept\n\nDescription.\n".to_string(),
                leaves: vec!["leaf-a.md".to_string(), "leaf-b.md".to_string()],
            }],
            leaf_assignments,
        };

        let summary =
            execute_plan(&plan, &cfg, &valid_filenames, "2025-06-01T12:00:00Z", &[]).unwrap();

        // Branch file written
        let branch_path = dir.path().join("branches").join("test-concept.md");
        assert!(branch_path.exists());
        let branch_content = fs::read_to_string(&branch_path).unwrap();
        assert!(branch_content.contains("title: Test Concept"));
        assert!(branch_content.contains("leaf-a.md"));
        assert!(branch_content.contains("leaf-b.md"));

        // Leaf frontmatter updated
        let leaf_a = fs::read_to_string(dir.path().join("leaf-a.md")).unwrap();
        assert!(leaf_a.contains("branches:"));
        assert!(leaf_a.contains("- test-concept"));
        assert!(leaf_a.contains("updated_at: 2025-06-01T12:00:00Z"));

        // Summary correct
        assert_eq!(summary.branches.len(), 1);
        assert_eq!(summary.leaves_updated, 2);
    }

    #[test]
    fn execute_plan_empty_branches_resets_leaf_frontmatter() {
        let dir = TempDir::new().unwrap();
        let cfg = make_test_config(dir.path());

        // Write a leaf that already has branches assigned
        fs::write(
            dir.path().join("leaf-a.md"),
            "---\ntitle: A\nurl: https://example.com/a\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\nbranches:\n  - old-branch\n---\n\n# A\n\nBody.\n",
        ).unwrap();

        let valid_filenames: HashSet<String> =
            ["leaf-a.md"].iter().map(|s| s.to_string()).collect();

        let plan = CompilePlan {
            branches: Vec::new(),
            leaf_assignments: HashMap::new(),
        };

        execute_plan(&plan, &cfg, &valid_filenames, "2025-06-01T12:00:00Z", &[]).unwrap();

        let content = fs::read_to_string(dir.path().join("leaf-a.md")).unwrap();
        assert!(content.contains("branches: []"));
    }

    // ── build_user_message test ───────────────────────────────────────────────

    #[test]
    fn build_user_message_uses_xml_fencing() {
        let leaves = vec![LoadedLeaf {
            filename: "test.md".to_string(),
            title: "Test Doc".to_string(),
            body: "Some body content.".to_string(),
        }];

        let msg = build_user_message(&leaves);
        assert!(msg.contains("<document filename=\"test.md\" title=\"Test Doc\">"));
        assert!(msg.contains("Some body content."));
        assert!(msg.contains("</document>"));
    }

    // ── compile_response_schema test ──────────────────────────────────────────

    #[test]
    fn schema_is_valid_json_schema() {
        let schema = compile_response_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["branches"].is_object());
        assert_eq!(schema["properties"]["branches"]["type"], "array");
    }
}
