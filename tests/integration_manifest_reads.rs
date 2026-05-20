// Integration tests for manifest-only reads after the 3b cutover.
//
// With 3b, `.bo/manifest.json` is the only tree-state store. Legacy
// `index.jsonl`/`state.json` mirrors are not written, and deleting the manifest
// is an unrecoverable tree-state loss rather than a reconstruction trigger.
//
// Tests the full CLI binary with $HOME override. No network/LLM required —
// fixtures are staged on disk directly so we can invoke `bo status`, `bo list`,
// `bo show`, etc. without going through `collect`/`compile`.

use bo::domain::{Slug, Timestamp, Title};
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

fn seed(home: &Path, output_dir: &Path) -> Output {
    bo(home)
        .args(["seed", output_dir.to_str().unwrap()])
        .output()
        .expect("failed to run bo seed")
}

fn run(home: &Path, args: &[&str]) -> Output {
    bo(home)
        .args(args)
        .output()
        .expect("failed to run bo command")
}

/// Stage a tree containing a fully populated manifest, three leaf files,
/// and one branch file. The fixture mimics the manifest-only outcome of
/// normal seed → collect → compile.
fn stage_tree(home: &Path) -> std::path::PathBuf {
    let tree_dir = home.join("tree");
    seed(home, &tree_dir);

    let leaves = [
        ("alpha", "Alpha", "https://example.com/alpha"),
        ("beta", "Beta", "https://example.com/beta"),
        ("gamma", "Gamma", "https://example.com/gamma"),
    ];

    // Leaf .md files
    for (slug, title, url) in &leaves {
        let content = format!(
            "---\ntitle: \"{title}\"\nurl: {url}\ncollected_at: 2026-01-01T00:00:00Z\nupdated_at: 2026-01-01T00:00:00Z\n---\n\n# {title}\n\nBody for {slug}.\n"
        );
        fs::write(tree_dir.join(format!("{slug}.md")), content).unwrap();
    }

    // Branch .md file
    let branches_dir = tree_dir.join("branches");
    fs::create_dir_all(&branches_dir).unwrap();
    let branch_content = "---\ntitle: \"topic-x\"\ncreated_at: 2026-01-02T10:00:00Z\nupdated_at: 2026-01-02T10:00:00Z\nleaves:\n  - alpha.md\n  - beta.md\n---\n\n# topic-x\n\nBranch body.\n";
    fs::write(branches_dir.join("topic-x.md"), branch_content).unwrap();

    // manifest.json — primary. Mirrors leaves + the one branch above with
    // last_compiled_at set so 'gamma' (collected 2026-01-01) shows as compiled.
    let m = bo::domain::manifest::Manifest {
        tree: bo::domain::manifest::TreeMeta {
            name: "tree".to_string(),
            created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2026-01-02T10:00:00Z").unwrap()),
        },
        leaves: leaves
            .iter()
            .map(|(slug, title, url)| bo::domain::manifest::LeafRecord {
                slug: Slug::parse(slug).unwrap(),
                file: format!("{slug}.md"),
                title: Title::new(*title),
                url: bo::domain::Url::parse(url).unwrap(),
                collected_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                summary: None,
            })
            .collect(),
        branches: vec![bo::domain::manifest::BranchRecord {
            slug: Slug::parse("topic-x").unwrap(),
            file: "branches/topic-x.md".to_string(),
            title: Title::new("topic-x"),
            created_at: Timestamp::parse("2026-01-02T10:00:00Z").unwrap(),
            updated_at: Timestamp::parse("2026-01-02T10:00:00Z").unwrap(),
            stale: false,
            leaves: vec![Slug::parse("alpha").unwrap(), Slug::parse("beta").unwrap()],
        }],
    };
    bo::domain::manifest::write(&tree_dir.join(".bo/manifest.json"), &m).unwrap();

    tree_dir
}

fn parse_data(output: &Output) -> Value {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("output is JSON");
    json["data"].clone()
}

#[test]
fn manifest_only_reads_work_without_secondary_store() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = stage_tree(tmp.path());

    assert!(!tree_dir.join(".bo/index.jsonl").exists());
    assert!(!tree_dir.join(".bo/state.json").exists());

    let status = parse_data(&run(tmp.path(), &["status", "--json"]));
    let list = parse_data(&run(tmp.path(), &["list", "--json"]));
    let show = parse_data(&run(tmp.path(), &["show", "Alpha", "--json"]));

    assert_eq!(status["leaves"]["total"], 3);
    assert_eq!(list["total_leaves"], 3);
    assert_eq!(show["leaf"]["file"], "alpha.md");
}

#[test]
fn missing_manifest_is_not_reconstructed() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = stage_tree(tmp.path());

    fs::remove_file(tree_dir.join(".bo/manifest.json")).unwrap();
    assert!(!tree_dir.join(".bo/manifest.json").exists());

    let out = run(tmp.path(), &["status", "--json"]);
    assert!(!out.status.success());

    let stdout = String::from_utf8_lossy(&out.stdout);
    let json: Value = serde_json::from_str(&stdout).expect("output is JSON");
    assert_eq!(json["error"]["code"], "io_error");
    assert!(!tree_dir.join(".bo/manifest.json").exists());
}
