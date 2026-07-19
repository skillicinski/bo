// Integration tests for state-only reads after the v0.1.0 cutover.
//
// `.bo/state.json` is the only tree-state store. Deleting it is an
// unrecoverable tree-state loss rather than a reconstruction trigger.
//
// Tests the full CLI binary with $HOME override. No network/LLM required —
// fixtures are staged on disk directly so we can invoke `bo status`, `bo list`,
// `bo show`, etc. without going through `collect`/`compile`.

mod common;

use bo::domain::{Slug, Timestamp, Title, Url};
use common::bo;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Output;
use tempfile::TempDir;

fn run(home: &Path, args: &[&str]) -> Output {
    bo(home)
        .args(args)
        .output()
        .expect("failed to run bo command")
}

/// Stage a tree containing a fully populated state, three leaf files,
/// and one branch file. The fixture mimics the state-only outcome of
/// normal seed -> collect -> compile.
fn stage_tree(home: &Path) -> std::path::PathBuf {
    let tree_dir = common::seed(home, "tree");

    let leaves = [
        ("alpha", "Alpha", "https://example.com/alpha"),
        ("beta", "Beta", "https://example.com/beta"),
        ("gamma", "Gamma", "https://example.com/gamma"),
    ];

    // Leaf .md files
    let leaves_dir = tree_dir.join("leaf");
    fs::create_dir_all(&leaves_dir).unwrap();
    for (slug, title, url) in &leaves {
        let content = format!(
            "---\ntitle: \"{title}\"\nurl: {url}\ncollected_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\n---\n\n# {title}\n\nBody for {slug}.\n"
        );
        fs::write(leaves_dir.join(format!("{slug}.md")), content).unwrap();
    }

    // Branch .md file
    let branches_dir = tree_dir.join("branch");
    fs::create_dir_all(&branches_dir).unwrap();
    let branch_content = "---\ntitle: \"topic-x\"\ncreated_at: 2026-01-02T10:00:00Z\nupdated_at: 2026-01-02T10:00:00Z\nleaves:\n  - alpha.md\n  - beta.md\n---\n\n# topic-x\n\nBranch body.\n";
    fs::write(branches_dir.join("topic-x.md"), branch_content).unwrap();

    // state.json — the only tree-state store. Mirrors leaves + the one branch
    // above with last_compiled_at set so 'gamma' (collected 2026-01-01) shows
    // as compiled.
    let state = bo::domain::state::TreeState {
        tree: bo::domain::state::TreeMetadata {
            name: "tree".to_string(),
            created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2026-01-02T10:00:00Z").unwrap()),
        },
        leaves: leaves
            .iter()
            .map(|(slug, title, url)| bo::domain::Leaf {
                slug: Slug::parse(slug).unwrap(),
                file: format!("leaf/{slug}.md"),
                title: Title::parse(title).ok(),
                url: Url::parse(url).unwrap(),
                collected_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                summary: None,
            })
            .collect(),
        branches: vec![bo::domain::Branch {
            slug: Slug::parse("topic-x").unwrap(),
            file: "branch/topic-x.md".to_string(),
            title: Title::parse("topic-x").unwrap(),
            created_at: Timestamp::parse("2026-01-02T10:00:00Z").unwrap(),
            updated_at: Timestamp::parse("2026-01-02T10:00:00Z").unwrap(),
            leaves: vec![Slug::parse("alpha").unwrap(), Slug::parse("beta").unwrap()],
        }],
    };
    common::write_state(&tree_dir, &state);

    tree_dir
}

fn parse_data(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("output is JSON");
    json["data"].clone()
}

/// Parse the JSON error envelope emitted on stderr for a failing --json command.
fn parse_error(output: &Output) -> Value {
    let stderr = String::from_utf8_lossy(&output.stderr);
    serde_json::from_str(&stderr).expect("stderr is a JSON error envelope")
}

#[test]
fn state_reads_work_without_secondary_store() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = stage_tree(tmp.path());

    // No legacy secondary store is written.
    assert!(!tree_dir.join(".bo/index.jsonl").exists());
    assert!(tree_dir.join(".bo/state.json").exists());

    let status = parse_data(&run(tmp.path(), &["status", "--json"]));
    let list = parse_data(&run(tmp.path(), &["list", "--json"]));
    let show = parse_data(&run(tmp.path(), &["show", "Alpha", "--json"]));

    assert_eq!(status["leaves"]["total"], 3);
    assert_eq!(list["total_leaves"], 3);
    assert_eq!(show["file"], "leaf/alpha.md");
}

#[test]
fn missing_state_is_not_reconstructed() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = stage_tree(tmp.path());

    fs::remove_file(tree_dir.join(".bo/state.json")).unwrap();
    assert!(!tree_dir.join(".bo/state.json").exists());

    let out = run(tmp.path(), &["status", "--json"]);
    assert!(!out.status.success());

    let json = parse_error(&out);
    assert_eq!(json["error"]["code"], "state_error");

    // No reconstruction: state.json is still absent.
    assert!(!tree_dir.join(".bo/state.json").exists());
}
