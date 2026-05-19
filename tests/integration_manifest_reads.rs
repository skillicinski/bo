// Integration tests for the manifest read-path migration.
//
// T7.1: with a manifest present, deleting the secondary store
//       (.bo/index.jsonl, .bo/state.json) does not change read output.
// T7.2: with the manifest absent, reconstruction from the secondary store
//       silently produces equivalent output and writes the manifest back
//       to disk with a one-line stderr warning.
//
// Tests the full CLI binary with $HOME override. No network/LLM required —
// fixtures are staged on disk directly so we can invoke `bo status`, `bo list`,
// `bo show`, etc. without going through `collect`/`compile`.

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
/// one branch file, and matching secondary-store mirrors (index.jsonl,
/// state.json). The fixture mimics the dual-write outcome of normal
/// seed → collect → compile.
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

    // index.jsonl (secondary)
    let index_lines = leaves
        .iter()
        .map(|(slug, title, url)| {
            format!("{{\"file\":\"{slug}.md\",\"title\":\"{title}\",\"url\":\"{url}\"}}")
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(tree_dir.join(".bo/index.jsonl"), format!("{index_lines}\n")).unwrap();

    // state.json (secondary)
    let state = serde_json::json!({
        "compiled_leaves": {
            "alpha": "2026-01-02T10:00:00Z",
            "beta":  "2026-01-02T10:00:00Z",
            "gamma": "2026-01-02T10:00:00Z",
        }
    });
    fs::write(
        tree_dir.join(".bo/state.json"),
        serde_json::to_string_pretty(&state).unwrap(),
    )
    .unwrap();

    // manifest.json — primary. Mirrors leaves + the one branch above with
    // last_compiled_at set so 'gamma' (collected 2026-01-01) shows as compiled.
    let m = bo::domain::manifest::Manifest {
        tree: bo::domain::manifest::TreeMeta {
            name: "tree".to_string(),
            created_at: "2026-01-01T00:00:00Z".to_string(),
            last_compiled_at: Some("2026-01-02T10:00:00Z".to_string()),
        },
        leaves: leaves
            .iter()
            .map(|(slug, title, url)| bo::domain::manifest::LeafRecord {
                slug: (*slug).to_string(),
                file: format!("{slug}.md"),
                title: (*title).to_string(),
                url: (*url).to_string(),
                collected_at: "2026-01-01T00:00:00Z".to_string(),
                summary: None,
            })
            .collect(),
        branches: vec![bo::domain::manifest::BranchRecord {
            slug: "topic-x".to_string(),
            file: "branches/topic-x.md".to_string(),
            title: "topic-x".to_string(),
            created_at: "2026-01-02T10:00:00Z".to_string(),
            updated_at: "2026-01-02T10:00:00Z".to_string(),
            stale: false,
            leaves: vec!["alpha".to_string(), "beta".to_string()],
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

// ── T7.1 ─────────────────────────────────────────────────────────────────────

#[test]
fn reads_survive_secondary_store_deletion() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = stage_tree(tmp.path());

    let status_before = parse_data(&run(tmp.path(), &["status", "--json"]));
    let list_before = parse_data(&run(tmp.path(), &["list", "--json"]));
    let show_before = parse_data(&run(tmp.path(), &["show", "Alpha", "--json"]));

    // Wipe the secondary store. The manifest is intact; reads should not care.
    fs::remove_file(tree_dir.join(".bo/index.jsonl")).unwrap();
    fs::remove_file(tree_dir.join(".bo/state.json")).unwrap();

    let status_after = parse_data(&run(tmp.path(), &["status", "--json"]));
    let list_after = parse_data(&run(tmp.path(), &["list", "--json"]));
    let show_after = parse_data(&run(tmp.path(), &["show", "Alpha", "--json"]));

    assert_eq!(
        status_before, status_after,
        "status output changed after deleting secondary store"
    );
    assert_eq!(
        list_before, list_after,
        "list output changed after deleting secondary store"
    );
    assert_eq!(
        show_before, show_after,
        "show output changed after deleting secondary store"
    );
}

// ── T7.2 ─────────────────────────────────────────────────────────────────────

#[test]
fn reads_survive_manifest_deletion_via_reconstruction() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = stage_tree(tmp.path());

    let status_control = parse_data(&run(tmp.path(), &["status", "--json"]));

    // Wipe the manifest. The secondary store carries enough information for
    // read_or_reconstruct to rebuild it on the next read.
    fs::remove_file(tree_dir.join(".bo/manifest.json")).unwrap();
    assert!(!tree_dir.join(".bo/manifest.json").exists());

    let out = run(tmp.path(), &["status", "--json"]);
    assert!(out.status.success());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("manifest missing; reconstructed from secondary store"),
        "expected reconstruction warning on stderr, got: {stderr}"
    );

    let status_recovered = parse_data(&out);
    assert_eq!(
        status_control, status_recovered,
        "status output diverged after reconstruction"
    );

    // Manifest was rewritten to disk.
    assert!(
        tree_dir.join(".bo/manifest.json").exists(),
        "reconstruction must persist a new manifest"
    );
}
