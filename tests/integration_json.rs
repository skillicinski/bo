// JSON envelope integration tests.
//
// Tests that all commands produce valid structured JSON output via --json flag.

use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn bo(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bo"));
    cmd.env("HOME", home);
    cmd
}

fn run(home: &Path, args: &[&str]) -> Output {
    bo(home).args(args).output().expect("failed to run bo")
}

fn parse_json(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout is not valid JSON: {e}\nstdout:\n{stdout}"))
}

fn seed_tree(home: &TempDir, name: &str) -> std::path::PathBuf {
    let output_dir = home.path().join(name);
    let out = run(home.path(), &["seed", output_dir.to_str().unwrap()]);
    assert!(out.status.success());
    output_dir
}

fn write_compile_leaf(tree: &Path, file: &str, title: &str) {
    bo::domain::index::append_entry(
        &tree.join("index.jsonl"),
        &bo::domain::index::IndexEntry {
            file: file.to_string(),
            title: title.to_string(),
            url: format!("https://example.com/{}", file.trim_end_matches(".md")),
        },
    )
    .unwrap();
    fs::write(
        tree.join(file),
        format!(
            "---\ntitle: {title}\nurl: https://example.com/{slug}\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# {title}\n\nBody.\n",
            slug = file.trim_end_matches(".md")
        ),
    )
    .unwrap();
}

// ── parse errors ─────────────────────────────────────────────────────────────

#[test]
fn json_parser_error_for_missing_subcommand() {
    let home = TempDir::new().unwrap();
    let out = run(home.path(), &["--json"]);
    assert!(!out.status.success());
    assert!(out.stderr.is_empty());
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["command"], "bo");
    assert_eq!(parsed["error"]["code"], "usage_error");
}

// ── flag positioning ─────────────────────────────────────────────────────────

#[test]
fn command_local_json_flag_is_accepted() {
    let home = TempDir::new().unwrap();
    let out = run(home.path(), &["list", "--json"]);
    assert!(!out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["command"], "list");
    assert_eq!(parsed["error"]["code"], "not_seeded");
}

#[test]
fn global_json_flag_is_accepted() {
    let home = TempDir::new().unwrap();
    let out = run(home.path(), &["--json", "list"]);
    assert!(!out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["command"], "list");
    assert_eq!(parsed["error"]["code"], "not_seeded");
}

// ── seed ─────────────────────────────────────────────────────────────────────

#[test]
fn seed_json_created_payload() {
    let home = TempDir::new().unwrap();
    let output_dir = home.path().join("my-tree");
    let out = run(
        home.path(),
        &["seed", "--json", output_dir.to_str().unwrap()],
    );
    assert!(out.status.success());
    assert!(out.stderr.is_empty());
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["command"], "seed");
    assert_eq!(parsed["data"]["status"], "created");
    assert_eq!(parsed["data"]["tree_name"], "my-tree");
}

#[test]
fn seed_json_already_seeded_payload() {
    let home = TempDir::new().unwrap();
    let output_dir = home.path().join("my-tree");
    let first = run(home.path(), &["seed", output_dir.to_str().unwrap()]);
    assert!(first.status.success());

    let out = run(
        home.path(),
        &["seed", "--json", output_dir.to_str().unwrap()],
    );
    assert!(out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["data"]["status"], "already_seeded");
    assert_eq!(parsed["data"]["tree_name"], "my-tree");
}

// ── every command ────────────────────────────────────────────────────────────

#[test]
fn every_output_command_accepts_json_flag() {
    let home = TempDir::new().unwrap();
    let output_dir = home.path().join("tree");

    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec!["seed", "--json", output_dir.to_str().unwrap()], "seed"),
        (vec!["collect", "--json", "https://example.com"], "collect"),
        (vec!["compile", "--json"], "compile"),
        (vec!["list", "--json"], "list"),
        (vec!["search", "--json", "term"], "search"),
        (vec!["show", "--json", "Title"], "show"),
        (vec!["raze", "--json"], "raze"),
    ];

    for (args, command) in cases {
        let h = TempDir::new().unwrap();
        let out = run(h.path(), &args);
        let parsed = parse_json(&out);
        assert_eq!(parsed["command"], command, "args: {args:?}");
        assert!(parsed.get("schema_version").is_some(), "args: {args:?}");
        assert!(parsed.get("warnings").is_some(), "args: {args:?}");
    }
}

// ── search ───────────────────────────────────────────────────────────────────

#[test]
fn search_json_no_results_exits_successfully() {
    let home = TempDir::new().unwrap();
    seed_tree(&home, "tree");

    let out = run(home.path(), &["search", "--json", "missing"]);
    assert!(out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["command"], "search");
    assert_eq!(parsed["data"]["hits"].as_array().unwrap().len(), 0);
    assert_eq!(parsed["data"]["query"]["terms"][0], "missing");
}

#[test]
fn search_json_page_zero_is_structured_usage_error() {
    let home = TempDir::new().unwrap();
    seed_tree(&home, "tree");

    let out = run(home.path(), &["search", "--json", "term", "--page", "0"]);
    assert_eq!(out.status.code(), Some(2));
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["command"], "search");
    assert_eq!(parsed["error"]["code"], "usage_error");
}

// ── compile ──────────────────────────────────────────────────────────────────

#[test]
fn compile_json_empty_tree_is_noop_without_api_key() {
    let home = TempDir::new().unwrap();
    let tree = seed_tree(&home, "tree");
    assert!(tree.exists());

    let out = bo(home.path())
        .args(["compile", "--json"])
        .env_remove("OPENAI_API_KEY")
        .output()
        .unwrap();
    assert!(out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["data"]["status"], "noop");
    assert_eq!(parsed["data"]["reason"], "empty_tree");
}

#[test]
fn compile_json_single_leaf_is_noop_without_api_key() {
    let home = TempDir::new().unwrap();
    let tree = seed_tree(&home, "tree");
    write_compile_leaf(&tree, "a.md", "A");

    let out = bo(home.path())
        .args(["compile", "--json"])
        .env_remove("OPENAI_API_KEY")
        .output()
        .unwrap();
    assert!(out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["data"]["status"], "noop");
    assert_eq!(parsed["data"]["reason"], "single_leaf");
}

#[test]
fn compile_json_missing_api_key_is_structured_error() {
    let home = TempDir::new().unwrap();
    let tree = seed_tree(&home, "tree");
    write_compile_leaf(&tree, "a.md", "A");
    write_compile_leaf(&tree, "b.md", "B");

    let out = bo(home.path())
        .args(["compile", "--json"])
        .env_remove("OPENAI_API_KEY")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["error"]["code"], "io_error");
    assert!(parsed["error"]["message"]
        .as_str()
        .unwrap()
        .contains("OPENAI_API_KEY"));
}

// ── show ─────────────────────────────────────────────────────────────────────

#[test]
fn show_json_not_found_is_structured_error() {
    let home = TempDir::new().unwrap();
    seed_tree(&home, "tree");

    let out = run(home.path(), &["show", "--json", "Missing"]);
    assert!(!out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["command"], "show");
    assert_eq!(parsed["error"]["code"], "not_found");
    assert_eq!(parsed["error"]["details"]["title"], "Missing");
}

#[test]
fn show_json_ambiguous_title_includes_candidates() {
    let home = TempDir::new().unwrap();
    let tree = seed_tree(&home, "tree");
    write_compile_leaf(&tree, "a.md", "Same Title");
    write_compile_leaf(&tree, "b.md", "Same Title");

    let out = run(home.path(), &["show", "--json", "Same Title"]);
    assert!(!out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["command"], "show");
    assert_eq!(parsed["error"]["code"], "ambiguous");
    assert_eq!(
        parsed["error"]["details"]["candidates"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

// ── raze ─────────────────────────────────────────────────────────────────────

#[test]
fn raze_json_summary() {
    let home = TempDir::new().unwrap();
    let tree = seed_tree(&home, "tree");

    let out = run(home.path(), &["raze", "--json"]);
    assert!(out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["command"], "raze");
    assert_eq!(parsed["data"]["output_dir"], tree.display().to_string());
    assert_eq!(parsed["data"]["removed_output_dir"], true);
    assert_eq!(parsed["data"]["deleted_config"], true);
}

#[test]
fn raze_json_reports_suspicious_ledger_entries_as_warnings() {
    let home = TempDir::new().unwrap();
    let tree = seed_tree(&home, "tree");
    bo::domain::index::append_entry(
        &tree.join("index.jsonl"),
        &bo::domain::index::IndexEntry {
            file: "../outside.md".to_string(),
            title: "Suspicious".to_string(),
            url: "https://example.com/suspicious".to_string(),
        },
    )
    .unwrap();

    let out = run(home.path(), &["raze", "--json"]);
    assert!(out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["warnings"][0]["code"], "suspicious_ledger_entry");
    assert_eq!(parsed["warnings"][0]["details"]["file"], "../outside.md");
}

// ── collect ──────────────────────────────────────────────────────────────────

#[test]
fn collect_json_duplicate_url_is_structured_error() {
    let home = TempDir::new().unwrap();
    let tree = seed_tree(&home, "tree");
    let url = "https://www.youtube.com/watch?v=a1mhk7mAetk";
    bo::domain::index::append_entry(
        &tree.join("index.jsonl"),
        &bo::domain::index::IndexEntry {
            file: "existing.md".to_string(),
            title: "Existing Video".to_string(),
            url: url.to_string(),
        },
    )
    .unwrap();

    let out = run(home.path(), &["collect", "--json", url]);
    assert!(!out.status.success());
    let parsed = parse_json(&out);
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["command"], "collect");
    assert_eq!(parsed["error"]["code"], "duplicate_url");
    assert_eq!(parsed["error"]["details"]["existing_file"], "existing.md");
}
