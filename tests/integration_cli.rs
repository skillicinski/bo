// CLI integration tests.
//
// Uses $HOME override to redirect config to a temp dir, avoiding any
// interaction with the real ~/.bo/config.json.

use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Output, Stdio};
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

fn list(home: &Path, args: &[&str]) -> Output {
    bo(home)
        .arg("list")
        .args(args)
        .output()
        .expect("failed to run bo list")
}

fn show(home: &Path, args: &[&str]) -> Output {
    bo(home)
        .arg("show")
        .args(args)
        .output()
        .expect("failed to run bo show")
}

fn raze(home: &Path) -> Output {
    bo(home)
        .arg("raze")
        .output()
        .expect("failed to run bo raze")
}

fn config_path(home: &TempDir) -> std::path::PathBuf {
    home.path().join(".bo").join("config.json")
}

fn auth_path(home: &TempDir) -> std::path::PathBuf {
    home.path().join(".bo").join("auth.json")
}

fn config_auth(home: &Path, args: &[&str], input: &str) -> Output {
    let mut child = bo(home)
        .arg("config")
        .arg("auth")
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run bo config auth");

    child
        .stdin
        .as_mut()
        .expect("stdin should be piped")
        .write_all(input.as_bytes())
        .expect("failed to write API key to stdin");

    child
        .wait_with_output()
        .expect("failed to wait for bo config auth")
}

fn append_index_entry(tree: &Path, file: &str, title: &str) {
    bo::domain::index::append_entry(
        &tree.join("index.jsonl"),
        &bo::domain::index::IndexEntry {
            file: file.to_string(),
            title: title.to_string(),
            url: format!("https://example.com/{}", file.trim_end_matches(".md")),
        },
    )
    .unwrap();
}

fn write_leaf(tree: &Path, file: &str, title: &str, collected_at: &str, branches: Option<&[&str]>) {
    let escaped_title = title.replace('\\', "\\\\").replace('"', "\\\"");
    let mut content = format!(
        "---\ntitle: \"{}\"\nurl: https://example.com/{}\ncollected_at: {}\nupdated_at: {}\n",
        escaped_title,
        file.trim_end_matches(".md"),
        collected_at,
        collected_at,
    );

    if let Some(branches) = branches {
        if branches.is_empty() {
            content.push_str("branches: []\n");
        } else {
            content.push_str("branches:\n");
            for branch in branches {
                content.push_str(&format!("  - {branch}\n"));
            }
        }
    }

    content.push_str(&format!("---\n\n# {title}\n\nBody.\n"));
    fs::write(tree.join(file), content).unwrap();
}

fn write_basic_list_tree(tree: &Path) {
    append_index_entry(tree, "beta-entry.md", "Beta Entry");
    write_leaf(
        tree,
        "beta-entry.md",
        "Beta Entry",
        "2025-01-05T08:00:00Z",
        Some(&[] as &[&str]),
    );

    append_index_entry(tree, "alpha-entry.md", "Alpha Entry");
    write_leaf(
        tree,
        "alpha-entry.md",
        "Alpha Entry",
        "2025-01-10T09:30:00Z",
        Some(&["branch_a", "branch_b"]),
    );

    append_index_entry(tree, "gamma-entry.md", "Gamma Entry");
    write_leaf(
        tree,
        "gamma-entry.md",
        "Gamma Entry",
        "2025-02-01T07:15:00Z",
        Some(&["branch_a_extra"]),
    );
}

fn write_json_list_tree(tree: &Path) {
    append_index_entry(tree, "live-entry.md", "Live Entry");
    write_leaf(
        tree,
        "live-entry.md",
        "Live Entry",
        "2025-03-01T12:00:00Z",
        Some(&["branch_a"]),
    );

    append_index_entry(tree, "missing-entry.md", "Missing Entry");
}

fn write_show_leaf(
    tree: &Path,
    file: &str,
    index_title: &str,
    frontmatter_title: &str,
    body: &str,
) {
    append_index_entry(tree, file, index_title);

    let escaped_title = frontmatter_title.replace('\\', "\\\\").replace('"', "\\\"");
    let content = format!(
        "---\ntitle: \"{}\"\nurl: https://example.com/{}\ncollected_at: 2025-04-01T12:00:00Z\nupdated_at: 2025-04-01T12:00:00Z\n---\n\n{}",
        escaped_title,
        file.trim_end_matches(".md"),
        body,
    );

    if let Some(parent) = tree.join(file).parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(tree.join(file), content).unwrap();
}

fn write_combined_flags_tree(tree: &Path) {
    append_index_entry(tree, "a-oldest.md", "A Oldest");
    write_leaf(
        tree,
        "a-oldest.md",
        "A Oldest",
        "2025-01-01T00:00:00Z",
        Some(&["branch_a"]),
    );

    append_index_entry(tree, "a-middle.md", "A Middle");
    write_leaf(
        tree,
        "a-middle.md",
        "A Middle",
        "2025-03-15T00:00:00Z",
        Some(&["branch_a"]),
    );

    append_index_entry(tree, "b-other.md", "B Other");
    write_leaf(
        tree,
        "b-other.md",
        "B Other",
        "2025-07-01T00:00:00Z",
        Some(&["branch_b"]),
    );

    append_index_entry(tree, "a-old.md", "A Old");
    write_leaf(
        tree,
        "a-old.md",
        "A Old",
        "2025-01-15T00:00:00Z",
        Some(&["branch_a"]),
    );

    append_index_entry(tree, "a-newest.md", "A Newest");
    write_leaf(
        tree,
        "a-newest.md",
        "A Newest",
        "2025-06-01T00:00:00Z",
        Some(&["branch_a"]),
    );

    append_index_entry(tree, "a-oldish.md", "A Oldish");
    write_leaf(
        tree,
        "a-oldish.md",
        "A Oldish",
        "2025-01-10T00:00:00Z",
        Some(&["branch_a"]),
    );

    append_index_entry(tree, "a-older.md", "A Older");
    write_leaf(
        tree,
        "a-older.md",
        "A Older",
        "2025-02-01T00:00:00Z",
        Some(&["branch_a"]),
    );
}

// ── seed ─────────────────────────────────────────────────────────────────────

#[test]
fn seed_creates_output_dir_and_config() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let out = seed(home.path(), &tree);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Output dir created
    assert!(tree.exists());

    // Config written and contains the path
    let cfg_path = config_path(&home);
    assert!(cfg_path.exists());
    let contents = fs::read_to_string(&cfg_path).unwrap();
    let parsed: Value = serde_json::from_str(&contents).unwrap();
    assert_eq!(parsed["tree"]["output_dir"], tree.to_str().unwrap());
}

#[test]
fn seed_already_seeded_is_idempotent() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let out1 = seed(home.path(), &tree);
    assert!(out1.status.success());

    // Second seed with same dir: succeeds (no error), prints already-seeded message
    let out2 = seed(home.path(), &tree);
    assert!(out2.status.success());
    let stdout = String::from_utf8_lossy(&out2.stdout);
    assert!(
        stdout.contains("already been seeded"),
        "expected already-seeded message, got: {stdout}"
    );

    // Config still valid after double seed
    let cfg_path = config_path(&home);
    let contents = fs::read_to_string(&cfg_path).unwrap();
    let parsed: Value = serde_json::from_str(&contents).unwrap();
    assert_eq!(parsed["tree"]["output_dir"], tree.to_str().unwrap());
}

#[test]
fn collect_without_seed_fails_with_helpful_message() {
    let home = TempDir::new().unwrap();

    let out = bo(home.path())
        .args(["collect", "https://example.com"])
        .output()
        .unwrap();

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("seed"),
        "expected hint to run seed, got: {stderr}"
    );
}

// ── list ─────────────────────────────────────────────────────────────────────

#[test]
fn list_without_seed_fails_with_existing_seed_hint() {
    let home = TempDir::new().unwrap();

    let out = list(home.path(), &[]);
    assert!(!out.status.success());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bo hasn't been seeded yet"),
        "expected existing seed hint, got: {stderr}"
    );
    assert!(stderr.contains("bo seed"), "stderr: {stderr}");
}

#[test]
fn list_on_seeded_empty_tree_reports_no_leaves_collected_yet() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(
        seeded.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&seeded.stderr)
    );

    let out = list(home.path(), &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("no leaves collected yet"),
        "stdout: {stdout}"
    );
}

#[test]
fn list_on_synthetic_tree_uses_index_order_and_shows_dates_and_branch_arrays() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    write_basic_list_tree(&tree);

    let out = list(home.path(), &[]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let beta_pos = stdout
        .find("Beta Entry")
        .expect("missing Beta Entry in output");
    let alpha_pos = stdout
        .find("Alpha Entry")
        .expect("missing Alpha Entry in output");
    let gamma_pos = stdout
        .find("Gamma Entry")
        .expect("missing Gamma Entry in output");

    assert!(beta_pos < alpha_pos, "stdout: {stdout}");
    assert!(alpha_pos < gamma_pos, "stdout: {stdout}");
    assert!(stdout.contains("2025-01-05"), "stdout: {stdout}");
    assert!(stdout.contains("2025-01-10"), "stdout: {stdout}");
    assert!(stdout.contains("2025-02-01"), "stdout: {stdout}");
    assert!(stdout.contains("[branch_a, branch_b]"), "stdout: {stdout}");
    assert!(stdout.contains("[]"), "stdout: {stdout}");
}

#[test]
fn list_limit_one_prints_at_most_one_leaf_title() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    write_basic_list_tree(&tree);

    let out = list(home.path(), &["--limit", "1"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let hits = ["Alpha Entry", "Beta Entry", "Gamma Entry"]
        .iter()
        .filter(|title| stdout.contains(**title))
        .count();

    assert_eq!(hits, 1, "expected exactly one listed title, got: {stdout}");
}

#[test]
fn list_branch_filter_is_exact_and_missing_branch_is_not_an_error() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    write_basic_list_tree(&tree);

    let out = list(home.path(), &["--branch", "branch_a"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Alpha Entry"), "stdout: {stdout}");
    assert!(!stdout.contains("Beta Entry"), "stdout: {stdout}");
    assert!(!stdout.contains("Gamma Entry"), "stdout: {stdout}");

    let missing = list(home.path(), &["--branch", "missing_branch"]);
    assert!(
        missing.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&missing.stderr)
    );

    let missing_stdout = String::from_utf8_lossy(&missing.stdout);
    assert!(
        missing_stdout.contains("no leaves matched branch 'missing_branch'"),
        "stdout: {missing_stdout}"
    );
}

#[test]
fn list_json_output_is_parseable_and_includes_required_fields_and_degradation_status() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    write_json_list_tree(&tree);

    let out = list(home.path(), &["--json"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let payload: Value = serde_json::from_slice(&out.stdout).expect("stdout was not valid JSON");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "list");
    let leaves = payload["data"]["leaves"]
        .as_array()
        .expect("expected enveloped JSON with data.leaves array");

    assert_eq!(leaves.len(), 2, "payload: {payload}");
    for row in leaves {
        assert!(row.get("file").is_some(), "row missing file: {row}");
        assert!(
            row.get("display_title").is_some(),
            "row missing display_title: {row}"
        );
        assert!(
            row.get("collected_at").is_some(),
            "row missing collected_at: {row}"
        );
        assert!(row.get("branches").is_some(), "row missing branches: {row}");
        assert!(row.get("degraded").is_some(), "row missing degraded: {row}");
        assert!(
            row.get("degradation_reasons").is_some(),
            "row missing degradation_reasons: {row}"
        );
    }

    let live = leaves
        .iter()
        .find(|row| row.get("file").and_then(Value::as_str) == Some("live-entry.md"))
        .expect("missing live-entry.md row");
    assert_eq!(live.get("degraded").and_then(Value::as_bool), Some(false));

    let missing = leaves
        .iter()
        .find(|row| row.get("file").and_then(Value::as_str) == Some("missing-entry.md"))
        .expect("missing missing-entry.md row");
    assert_eq!(missing.get("degraded").and_then(Value::as_bool), Some(true));
    assert!(
        missing
            .get("degradation_reasons")
            .and_then(Value::as_array)
            .is_some_and(|reasons| !reasons.is_empty()),
        "expected degradation reasons for missing row: {missing}"
    );
}

#[test]
fn list_combined_flags_filter_sort_limit_and_emit_json() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    write_combined_flags_tree(&tree);

    let out = list(
        home.path(),
        &["--branch", "branch_a", "--recent", "--limit", "5", "--json"],
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let payload: Value = serde_json::from_slice(&out.stdout).expect("stdout was not valid JSON");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "list");
    let leaves = payload["data"]["leaves"]
        .as_array()
        .expect("expected enveloped JSON with data.leaves array");

    assert_eq!(leaves.len(), 5, "payload: {payload}");

    let files: Vec<&str> = leaves
        .iter()
        .map(|row| {
            row.get("file")
                .and_then(Value::as_str)
                .expect("row missing file")
        })
        .collect();
    assert_eq!(
        files,
        vec![
            "a-newest.md",
            "a-middle.md",
            "a-older.md",
            "a-old.md",
            "a-oldish.md",
        ]
    );

    for row in leaves {
        let branches = row
            .get("branches")
            .and_then(Value::as_array)
            .expect("row missing branches array");
        assert!(
            branches
                .iter()
                .any(|branch| branch.as_str() == Some("branch_a")),
            "row missing branch_a: {row}"
        );
    }
    assert!(!files.contains(&"b-other.md"));
    assert!(!files.contains(&"a-oldest.md"));
}

// ── show ─────────────────────────────────────────────────────────────────────

#[test]
fn show_without_seed_fails_with_existing_seed_hint() {
    let home = TempDir::new().unwrap();

    let out = show(home.path(), &["Some Title"]);
    assert!(!out.status.success());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("bo hasn't been seeded yet"),
        "expected existing seed hint, got: {stderr}"
    );
    assert!(stderr.contains("bo seed"), "stderr: {stderr}");
}

#[test]
fn show_prints_frontmatter_and_bounded_preview() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    let body = format!("# Some Title\n\n{}\nTAIL_MARKER\n", "A".repeat(10_000));
    write_show_leaf(&tree, "some-title.md", "Some Title", "Some Title", &body);

    let out = show(home.path(), &["Some Title"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("---\ntitle: \"Some Title\""),
        "stdout: {stdout}"
    );
    assert!(stdout.contains("# Some Title"), "stdout: {stdout}");
    assert!(stdout.contains("preview truncated"), "stdout: {stdout}");
    assert!(!stdout.contains("TAIL_MARKER"), "stdout: {stdout}");
}

#[test]
fn show_title_matching_is_case_insensitive_and_exact() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    write_show_leaf(
        &tree,
        "case-title.md",
        "Case Title",
        "Case Title",
        "Body.\n",
    );

    let matched = show(home.path(), &["case title"]);
    assert!(
        matched.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&matched.stderr)
    );

    let partial = show(home.path(), &["Case"]);
    assert!(!partial.status.success());
    let stderr = String::from_utf8_lossy(&partial.stderr);
    assert!(stderr.contains("not found"), "stderr: {stderr}");
}

#[test]
fn show_full_prints_complete_body() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    let body = format!("# Full Title\n\n{}\nTAIL_MARKER\n", "B".repeat(10_000));
    write_show_leaf(&tree, "full-title.md", "Full Title", "Full Title", &body);

    let out = show(home.path(), &["--full", "Full Title"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("TAIL_MARKER"), "stdout: {stdout}");
    assert!(!stdout.contains("preview truncated"), "stdout: {stdout}");
}

#[test]
fn show_json_output_is_parseable_and_contains_required_fields() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    let body = format!("# Json Title\n\n{}\nTAIL_MARKER\n", "C".repeat(10_000));
    write_show_leaf(&tree, "json-title.md", "Json Title", "Json Title", &body);

    let out = show(home.path(), &["--json", "Json Title"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let payload: Value = serde_json::from_slice(&out.stdout).expect("stdout was not valid JSON");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "show");
    let leaf = payload["data"].get("leaf").expect("missing leaf object");

    assert_eq!(leaf["title"], "Json Title");
    assert_eq!(leaf["file"], "json-title.md");
    assert_eq!(leaf["url"], "https://example.com/json-title");
    assert_eq!(leaf["frontmatter"]["title"], "Json Title");
    assert!(
        leaf["frontmatter_raw"]
            .as_str()
            .is_some_and(|raw| raw.contains("title: \"Json Title\"")),
        "leaf: {leaf}"
    );
    assert_eq!(leaf["truncated"], true);
    assert_eq!(leaf["full"], false);
    assert!(
        !leaf["body"]
            .as_str()
            .expect("body must be a string")
            .contains("TAIL_MARKER"),
        "leaf: {leaf}"
    );
}

#[test]
fn show_json_full_output_contains_full_body() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    let body = format!("# Json Full\n\n{}\nTAIL_MARKER\n", "D".repeat(10_000));
    write_show_leaf(&tree, "json-full.md", "Json Full", "Json Full", &body);

    let out = show(home.path(), &["--json", "--full", "Json Full"]);
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let payload: Value = serde_json::from_slice(&out.stdout).expect("stdout was not valid JSON");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "show");
    let leaf = payload["data"].get("leaf").expect("missing leaf object");

    assert_eq!(leaf["truncated"], false);
    assert_eq!(leaf["full"], true);
    assert!(
        leaf["body"]
            .as_str()
            .expect("body must be a string")
            .contains("TAIL_MARKER"),
        "leaf: {leaf}"
    );
}

#[test]
fn show_missing_title_reports_not_found_and_suggests_list() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    write_show_leaf(&tree, "available.md", "Available", "Available", "Body.\n");

    let out = show(home.path(), &["Missing Title"]);
    assert!(!out.status.success());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not found"), "stderr: {stderr}");
    assert!(stderr.contains("bo list"), "stderr: {stderr}");
}

#[test]
fn show_duplicate_title_reports_ambiguity_with_candidates() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());
    write_show_leaf(&tree, "duplicate-a.md", "Duplicate", "Duplicate", "A\n");
    write_show_leaf(&tree, "duplicate-b.md", "Duplicate", "duplicate", "B\n");

    let out = show(home.path(), &["DUPLICATE"]);
    assert!(!out.status.success());

    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("ambiguous"), "stderr: {stderr}");
    assert!(stderr.contains("duplicate-a.md"), "stderr: {stderr}");
    assert!(stderr.contains("duplicate-b.md"), "stderr: {stderr}");
}

// ── config auth ──────────────────────────────────────────────────────────────

#[test]
fn config_auth_stores_openai_auth_separately_from_config() {
    let home = TempDir::new().unwrap();
    let secret = "sk-config-auth-one";

    let out = config_auth(
        home.path(),
        &["--provider", "openai"],
        &format!("{secret}\n"),
    );
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stdout.contains("openai auth configured"),
        "stdout: {stdout}"
    );
    assert!(!stdout.contains(secret));
    assert!(!stderr.contains(secret));

    assert!(auth_path(&home).exists());
    assert!(!config_path(&home).exists());

    let auth: Value = serde_json::from_str(&fs::read_to_string(auth_path(&home)).unwrap()).unwrap();
    assert_eq!(auth["providers"]["openai"]["api_key"], secret);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(auth_path(&home)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[test]
fn config_auth_overwrites_existing_key_without_printing_either_key() {
    let home = TempDir::new().unwrap();
    let first = "sk-config-auth-first";
    let second = "sk-config-auth-second";

    let first_out = config_auth(
        home.path(),
        &["--provider", "openai"],
        &format!("{first}\n"),
    );
    assert!(first_out.status.success());

    let second_out = config_auth(
        home.path(),
        &["--provider", "openai"],
        &format!("{second}\n"),
    );
    assert!(second_out.status.success());

    let auth = fs::read_to_string(auth_path(&home)).unwrap();
    assert!(auth.contains(second));
    assert!(!auth.contains(first));

    for output in [&first_out, &second_out] {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stdout.contains(first));
        assert!(!stdout.contains(second));
        assert!(!stderr.contains(first));
        assert!(!stderr.contains(second));
    }
}

#[test]
fn config_auth_json_success_has_expected_shape_and_no_secret() {
    let home = TempDir::new().unwrap();
    let secret = "sk-config-auth-json";
    let mut child = bo(home.path())
        .args(["--json", "config", "auth", "--provider", "openai"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to run bo config auth");

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(format!("{secret}\n").as_bytes())
        .unwrap();

    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(!stdout.contains(secret));
    assert!(!stderr.contains(secret));

    let payload: Value = serde_json::from_slice(&out.stdout).expect("stdout was not valid JSON");
    assert_eq!(payload["ok"], true);
    assert_eq!(payload["command"], "config");
    assert_eq!(payload["data"]["status"], "ok");
    assert_eq!(payload["data"]["provider"], "openai");
    assert_eq!(payload["data"]["auth"], "configured");
    assert!(payload["data"].get("api_key").is_none());
}

#[test]
fn config_auth_unknown_provider_exits_2_and_does_not_prompt() {
    let home = TempDir::new().unwrap();

    let out = bo(home.path())
        .args(["config", "auth", "--provider", "OpenAI"])
        .output()
        .expect("failed to run bo config auth");

    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown provider"), "stderr: {stderr}");
    assert!(stderr.contains("openai"), "stderr: {stderr}");
    assert!(!stderr.contains("OpenAI API key"), "stderr: {stderr}");
    assert!(!auth_path(&home).exists());
}

#[test]
fn config_auth_json_unknown_provider_lists_valid_providers() {
    let home = TempDir::new().unwrap();

    let out = bo(home.path())
        .args(["--json", "config", "auth", "--provider", "unknown"])
        .output()
        .expect("failed to run bo config auth");

    assert_eq!(out.status.code(), Some(2));
    let payload: Value = serde_json::from_slice(&out.stdout).expect("stdout was not valid JSON");
    assert_eq!(payload["ok"], false);
    assert_eq!(payload["command"], "config");
    assert_eq!(payload["error"]["code"], "usage_error");
    assert_eq!(payload["error"]["details"]["valid_providers"][0], "openai");
    assert!(!String::from_utf8_lossy(&out.stderr).contains("OpenAI API key"));
}

// ── query auth ──────────────────────────────────────────────────────────────

#[test]
fn query_without_auth_points_to_config_auth() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");
    let seeded = seed(home.path(), &tree);
    assert!(seeded.status.success());

    let out = bo(home.path())
        .args(["query", "what", "is", "collected?"])
        .env_remove("OPENAI_API_KEY")
        .output()
        .expect("failed to run bo query");

    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("OpenAI API key not configured. Run: bo config auth --provider openai"),
        "stderr: {stderr}"
    );
}

// ── raze ─────────────────────────────────────────────────────────────────────

#[test]
fn raze_removes_config_auth_and_cleans_tree() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    // Seed
    seed(home.path(), &tree);
    assert!(config_path(&home).exists());
    fs::create_dir_all(auth_path(&home).parent().unwrap()).unwrap();
    fs::write(
        auth_path(&home),
        r#"{"providers":{"openai":{"api_key":"sk-raze"}}}"#,
    )
    .unwrap();
    assert!(auth_path(&home).exists());

    // Manually write a tree file and index entry so raze has something to delete
    fs::create_dir_all(&tree).unwrap();
    fs::write(tree.join("article.md"), "# Article").unwrap();
    let entry = serde_json::json!({
        "file": "article.md",
        "title": "Article",
        "url": "https://example.com/article"
    });
    fs::write(
        tree.join("index.jsonl"),
        serde_json::to_string(&entry).unwrap(),
    )
    .unwrap();

    let out = raze(home.path());
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Config and auth deleted
    assert!(!config_path(&home).exists());
    assert!(!auth_path(&home).exists());

    // Tree file and index deleted
    assert!(!tree.join("article.md").exists());
    assert!(!tree.join("index.jsonl").exists());
}

#[test]
fn raze_without_seed_fails_with_helpful_message() {
    let home = TempDir::new().unwrap();

    let out = raze(home.path());
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("seed"),
        "expected hint to run seed, got: {stderr}"
    );
}

#[test]
fn raze_auth_only_deletes_auth_and_succeeds_without_seed() {
    let home = TempDir::new().unwrap();
    fs::create_dir_all(auth_path(&home).parent().unwrap()).unwrap();
    fs::write(
        auth_path(&home),
        r#"{"providers":{"openai":{"api_key":"sk-auth-only"}}}"#,
    )
    .unwrap();

    let out = raze(home.path());
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stdout.contains("deleted auth"), "stdout: {stdout}");
    assert!(!stdout.contains("sk-auth-only"));
    assert!(!stderr.contains("sk-auth-only"));
    assert!(!auth_path(&home).exists());
}

#[test]
fn raze_tolerates_already_deleted_files() {
    let home = TempDir::new().unwrap();
    let tree = home.path().join("my-tree");

    seed(home.path(), &tree);

    // Ledger references a file that doesn't exist on disk
    fs::create_dir_all(&tree).unwrap();
    let entry = serde_json::json!({
        "file": "gone.md",
        "title": "Gone",
        "url": "https://example.com/gone"
    });
    fs::write(
        tree.join("index.jsonl"),
        serde_json::to_string(&entry).unwrap(),
    )
    .unwrap();

    // Should not error — missing files are silently skipped
    let out = raze(home.path());
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!config_path(&home).exists());
}
