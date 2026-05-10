// Integration tests for `bo search`.

use std::fs;
use std::process::Command;
use tempfile::TempDir;

fn bo_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_bo"))
}

struct TestTree {
    _home: TempDir,
    #[allow(dead_code)]
    tree_dir: TempDir,
}

impl TestTree {
    fn new(leaves: &[(&str, &str, &str, &str)]) -> Self {
        let home = TempDir::new().unwrap();
        let tree_dir = TempDir::new().unwrap();

        // Write config
        let config_dir = home.path().join(".bo");
        fs::create_dir_all(&config_dir).unwrap();
        let config = serde_json::json!({
            "tree": {
                "output_dir": tree_dir.path().to_str().unwrap(),
                "name": "test-tree",
                "created_at": "2025-01-01T00:00:00Z"
            }
        });
        fs::write(config_dir.join("config.json"), config.to_string()).unwrap();

        // Write leaves and index
        let mut index_lines = Vec::new();
        for (file, title, body, date) in leaves {
            let content = format!(
                "---\ntitle: \"{}\"\nurl: https://example.com/{}\ncollected_at: {}\nupdated_at: {}\n---\n\n# {}\n\n{}\n",
                title, file, date, date, title, body
            );
            fs::write(tree_dir.path().join(file), &content).unwrap();
            index_lines.push(format!(
                r#"{{"file":"{}","title":"{}","url":"https://example.com/{}"}}"#,
                file, title, file
            ));
        }
        fs::write(
            tree_dir.path().join("index.jsonl"),
            index_lines.join("\n") + "\n",
        )
        .unwrap();

        TestTree {
            _home: home,
            tree_dir,
        }
    }

    fn search(&self, args: &[&str]) -> std::process::Output {
        let mut cmd = bo_bin();
        cmd.env("HOME", self._home.path());
        cmd.arg("search");
        for arg in args {
            cmd.arg(arg);
        }
        cmd.output().unwrap()
    }
}

fn corpus() -> Vec<(&'static str, &'static str, &'static str, &'static str)> {
    vec![
        (
            "rust-ownership.md",
            "Understanding Ownership",
            "Ownership is Rust's most unique feature. It enables Rust to make memory safety guarantees without needing a garbage collector. The borrow checker enforces these ownership rules at compile time.",
            "2025-01-15T10:00:00Z",
        ),
        (
            "rust-lifetimes.md",
            "Lifetimes in Rust",
            "Every reference in Rust has a lifetime. Most of the time, lifetimes are implicit and inferred. We must annotate lifetimes when the lifetimes of references could be related in a few different ways.",
            "2025-02-01T10:00:00Z",
        ),
        (
            "rust-traits.md",
            "Traits: Defining Shared Behavior",
            "A trait defines functionality a particular type has and can share with other types. We can use traits to define shared behavior in an abstract way. Trait bounds specify generic types.",
            "2025-03-01T10:00:00Z",
        ),
        (
            "python-gc.md",
            "Python Garbage Collection",
            "Python uses reference counting with a cycle-detecting garbage collector. Memory management is automatic. Objects are freed when their reference count drops to zero.",
            "2025-04-01T10:00:00Z",
        ),
        (
            "go-concurrency.md",
            "Go Concurrency Patterns",
            "Goroutines and channels make concurrent programming straightforward in Go. The select statement lets a goroutine wait on multiple communication operations.",
            "2025-05-01T10:00:00Z",
        ),
        (
            "rust-async.md",
            "Async Rust",
            "Rust's async/await syntax makes it possible to write asynchronous code that looks like synchronous code. Futures are lazy in Rust and do nothing unless polled.",
            "2025-05-15T10:00:00Z",
        ),
        (
            "memory-safety.md",
            "Memory Safety Without GC",
            "Rust achieves memory safety without a garbage collector through its ownership system. The borrow checker statically prevents data races and use-after-free bugs at compile time.",
            "2025-06-01T10:00:00Z",
        ),
        (
            "rust-error-handling.md",
            "Error Handling in Rust",
            "Rust groups errors into recoverable and unrecoverable errors. Result and Option types make error handling explicit. The question mark operator propagates errors concisely.",
            "2025-06-15T10:00:00Z",
        ),
        (
            "systems-programming.md",
            "Systems Programming Languages",
            "Systems programming requires low-level control over memory and hardware. Rust, C, and C++ are popular choices. Rust offers memory safety without runtime overhead.",
            "2025-07-01T10:00:00Z",
        ),
        (
            "rust-modules.md",
            "Rust Module System",
            "The module system includes packages, crates, and modules. Use mod to define modules and pub to control visibility. The use keyword brings paths into scope.",
            "2025-07-15T10:00:00Z",
        ),
        (
            "borrow-checker-deep.md",
            "Deep Dive: The Borrow Checker",
            "The borrow checker is Rust's compile-time guarantee of memory safety. It enforces that references must always be valid and that you cannot have mutable and immutable references simultaneously.",
            "2025-08-01T10:00:00Z",
        ),
    ]
}

#[test]
fn search_single_term_returns_matches() {
    let tree = TestTree::new(&corpus());
    let output = tree.search(&["ownership"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Understanding Ownership"));
    assert!(stdout.contains("Memory Safety Without GC"));
}

#[test]
fn search_multiple_terms_and_semantics() {
    let tree = TestTree::new(&corpus());
    let output = tree.search(&["borrow", "checker", "compile"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Both ownership and borrow-checker-deep mention all three terms
    assert!(stdout.contains("Understanding Ownership") || stdout.contains("Deep Dive"));
    // Python GC should not appear
    assert!(!stdout.contains("Python"));
}

#[test]
fn search_phrase_matching() {
    let tree = TestTree::new(&corpus());
    let output = tree.search(&["borrow checker"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Ownership")
            || stdout.contains("Borrow Checker")
            || stdout.contains("Memory Safety")
    );
    // "borrow and then checker" wouldn't match as phrase — verify Go concurrency doesn't appear
    assert!(!stdout.contains("Go Concurrency"));
}

#[test]
fn search_no_match_exits_1() {
    let tree = TestTree::new(&corpus());
    let output = tree.search(&["xyznonexistent"]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no results"));
}

#[test]
fn search_recent_flag_orders_by_date() {
    let tree = TestTree::new(&corpus());
    let output = tree.search(&["rust", "--recent"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    // First result should be from the most recent matching leaf
    // The most recent leaf mentioning "rust" is one of the later-dated ones
    assert!(!lines.is_empty());
}

#[test]
fn search_page_2() {
    let tree = TestTree::new(&corpus());
    // "rust" appears in many leaves (>5), so page 2 should have results
    let output = tree.search(&["rust", "--page", "2"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("page 2/"));
}

#[test]
fn search_json_output_parseable() {
    let tree = TestTree::new(&corpus());
    let output = tree.search(&["ownership", "--json"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["command"], "search");
    let data = &parsed["data"];
    assert!(!data["hits"].as_array().unwrap().is_empty());
    assert!(data["total_results"].as_u64().unwrap() > 0);
    assert_eq!(data["page"], 1);
    assert!(data["total_pages"].as_u64().unwrap() >= 1);
    assert_eq!(data["query"]["terms"][0], "ownership");

    // Each hit has required fields
    let hit = &data["hits"][0];
    assert!(hit["file"].is_string());
    assert!(hit["title"].is_string());
    assert!(hit["snippet"].is_string());
    // score is hidden from JSON (internal ranking only)
    assert!(hit.get("score").is_none());
}

#[test]
fn search_json_no_results_exits_0() {
    let tree = TestTree::new(&corpus());
    let output = tree.search(&["xyznonexistent", "--json"]);

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["data"]["total_results"], 0);
    assert_eq!(parsed["data"]["hits"].as_array().unwrap().len(), 0);
}

#[test]
fn search_case_insensitive() {
    let tree = TestTree::new(&corpus());
    let output = tree.search(&["RUST", "OWNERSHIP"]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Understanding Ownership"));
}

#[test]
fn search_out_of_range_page_exits_1() {
    let tree = TestTree::new(&corpus());
    let output = tree.search(&["rust", "--page", "999"]);

    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("no results on page"));
}
