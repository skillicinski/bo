use super::*;
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;
use std::time::SystemTime;
use tempfile::TempDir;

#[derive(Debug, Clone, Eq, PartialEq)]
struct FileSnapshot {
    len: u64,
    modified: Option<SystemTime>,
    contents: String,
}

#[test]
fn empty_index_returns_not_found_with_list_suggestion() {
    let dir = TempDir::new().unwrap();
    let err = show_leaf(dir.path(), "Missing", &ShowOptions::default()).unwrap_err();

    let message = err.to_string();
    assert!(message.contains("not found"), "message: {message}");
    assert!(message.contains("bo list"), "message: {message}");
}

#[test]
fn suspicious_path_is_rejected_and_never_read() {
    let sandbox = TempDir::new().unwrap();
    let tree_dir = sandbox.path().join("tree");
    fs::create_dir_all(&tree_dir).unwrap();
    write_index(&tree_dir, &[("../outside.md", "Outside Title")]);
    fs::write(
        sandbox.path().join("outside.md"),
        "---\ntitle: Outside Title\n---\n\noutside\n",
    )
    .unwrap();

    let err = show_leaf(&tree_dir, "Outside Title", &ShowOptions::default()).unwrap_err();

    assert!(matches!(err, ShowError::SuspiciousPath { .. }));
    assert!(err.to_string().contains("suspicious path"));
}

#[test]
fn show_leaf_preserves_raw_frontmatter_and_body() {
    let dir = TempDir::new().unwrap();
    write_index(dir.path(), &[("raw.md", "Raw: Title")]);
    write_raw_file(
        dir.path(),
        "raw.md",
        "---\ntitle: \"Raw: Title\"\nurl: https://example.com\n---\n\n# Heading\n\nBody.\n",
    );

    let result = show_leaf(dir.path(), "raw: title", &ShowOptions::default()).unwrap();

    assert_eq!(
        result.frontmatter_raw,
        "---\ntitle: \"Raw: Title\"\nurl: https://example.com\n---\n"
    );
    assert_eq!(result.body, "# Heading\n\nBody.\n");
    assert_eq!(
        result.frontmatter.get("title").and_then(Value::as_str),
        Some("Raw: Title")
    );
}

#[test]
fn title_uses_frontmatter_then_index_fallback() {
    let dir = TempDir::new().unwrap();
    write_index(
        dir.path(),
        &[
            ("frontmatter.md", "Stale Index Title"),
            ("index.md", "Index Fallback Title"),
        ],
    );
    write_leaf(
        dir.path(),
        "frontmatter.md",
        "title: Frontmatter Title\n",
        "body\n",
    );
    write_leaf(dir.path(), "index.md", "title: \"\"\n", "body\n");

    let frontmatter = show_leaf(dir.path(), "frontmatter title", &ShowOptions::default()).unwrap();
    let index = show_leaf(dir.path(), "index fallback title", &ShowOptions::default()).unwrap();

    assert_eq!(frontmatter.title, "Frontmatter Title");
    assert_eq!(frontmatter.file, "frontmatter.md");
    assert_eq!(index.title, "Index Fallback Title");
    assert_eq!(index.file, "index.md");
}

#[test]
fn matching_is_case_insensitive_and_exact() {
    let dir = TempDir::new().unwrap();
    write_index(dir.path(), &[("leaf.md", "Some Title")]);
    write_leaf(dir.path(), "leaf.md", "title: Some Title\n", "body\n");

    let result = show_leaf(dir.path(), "sOmE tItLe", &ShowOptions::default()).unwrap();
    assert_eq!(result.file, "leaf.md");

    let err = show_leaf(dir.path(), "Some", &ShowOptions::default()).unwrap_err();
    assert!(matches!(err, ShowError::NotFound { .. }));

    let whitespace_err =
        show_leaf(dir.path(), " Some Title ", &ShowOptions::default()).unwrap_err();
    assert!(matches!(whitespace_err, ShowError::NotFound { .. }));
}

#[test]
fn not_found_mentions_requested_title_and_list() {
    let dir = TempDir::new().unwrap();
    write_index(dir.path(), &[("leaf.md", "Available")]);
    write_leaf(dir.path(), "leaf.md", "title: Available\n", "body\n");

    let err = show_leaf(dir.path(), "Missing Title", &ShowOptions::default()).unwrap_err();
    let message = err.to_string();

    assert!(message.contains("Missing Title"), "message: {message}");
    assert!(message.contains("bo list"), "message: {message}");
}

#[test]
fn duplicate_titles_return_ambiguity_with_candidate_details() {
    let dir = TempDir::new().unwrap();
    write_index(
        dir.path(),
        &[("one.md", "Duplicate"), ("two.md", "Duplicate")],
    );
    write_leaf(dir.path(), "one.md", "title: Duplicate\n", "one\n");
    write_leaf(dir.path(), "two.md", "title: duplicate\n", "two\n");

    let err = show_leaf(dir.path(), "DUPLICATE", &ShowOptions::default()).unwrap_err();
    let ShowError::Ambiguous { candidates, .. } = &err else {
        panic!("expected ambiguous error, got {err:?}");
    };

    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].file, "one.md");
    assert_eq!(candidates[1].file, "two.md");
    let message = err.to_string();
    assert!(message.contains("ambiguous"), "message: {message}");
    assert!(message.contains("one.md"), "message: {message}");
    assert!(message.contains("two.md"), "message: {message}");
}

#[test]
fn selected_leaf_failures_are_clear() {
    let dir = TempDir::new().unwrap();
    write_index(
        dir.path(),
        &[
            ("missing.md", "Missing"),
            ("unreadable.md", "Unreadable"),
            ("broken.md", "Broken"),
        ],
    );
    fs::create_dir(dir.path().join("unreadable.md")).unwrap();
    write_raw_file(
        dir.path(),
        "broken.md",
        "---\n: invalid: yaml\n---\n\nbody\n",
    );

    let missing = show_leaf(dir.path(), "Missing", &ShowOptions::default()).unwrap_err();
    assert!(matches!(missing, ShowError::MissingFile { .. }));
    assert!(missing.to_string().contains("missing file"));

    let unreadable = show_leaf(dir.path(), "Unreadable", &ShowOptions::default()).unwrap_err();
    assert!(matches!(unreadable, ShowError::UnreadableFile { .. }));
    assert!(unreadable.to_string().contains("unreadable file"));

    let broken = show_leaf(dir.path(), "Broken", &ShowOptions::default()).unwrap_err();
    assert!(matches!(broken, ShowError::InvalidFrontmatter { .. }));
    assert!(broken.to_string().contains("invalid frontmatter"));
}

#[test]
fn show_leaf_short_body_is_not_truncated() {
    let dir = TempDir::new().unwrap();
    write_index(dir.path(), &[("short.md", "Short")]);
    write_leaf(dir.path(), "short.md", "title: Short\n", "short body");

    let result = show_leaf(dir.path(), "Short", &ShowOptions::default()).unwrap();

    assert_eq!(result.body, "short body");
    assert!(!result.truncated);
}

#[test]
fn show_leaf_full_option_returns_full_body() {
    let dir = TempDir::new().unwrap();
    let long_body = format!("{}TAIL", "a".repeat(PREVIEW_CHAR_LIMIT + 10));
    write_index(dir.path(), &[("leaf.md", "Long")]);
    write_leaf(dir.path(), "leaf.md", "title: Long\n", &long_body);

    let preview = show_leaf(dir.path(), "Long", &ShowOptions { full: false }).unwrap();
    let full = show_leaf(dir.path(), "Long", &ShowOptions { full: true }).unwrap();

    assert!(preview.truncated);
    assert!(!preview.body.contains("TAIL"));
    assert!(!full.truncated);
    assert_eq!(full.body, long_body);
}

#[test]
fn show_leaf_is_read_only() {
    let dir = TempDir::new().unwrap();
    write_index(dir.path(), &[("leaf.md", "Leaf")]);
    write_leaf(dir.path(), "leaf.md", "title: Leaf\n", "body\n");
    let before = snapshot_tree(dir.path());

    let _ = show_leaf(dir.path(), "Leaf", &ShowOptions::default()).unwrap();

    let after = snapshot_tree(dir.path());
    assert_eq!(before, after);
}

#[test]
fn render_human_preview_includes_frontmatter_body_and_truncation_marker() {
    let result = fixture_result("preview body", true, false);

    let output = render_human(&result);

    assert!(
        output.contains("---\ntitle: Rendered\n---\n"),
        "output: {output}"
    );
    assert!(output.contains("preview body"), "output: {output}");
    assert!(output.contains("preview truncated"), "output: {output}");
}

#[test]
fn render_human_full_has_no_truncation_marker() {
    let result = fixture_result("complete body", false, true);

    let output = render_human(&result);

    assert!(output.contains("complete body"), "output: {output}");
    assert!(!output.contains("preview truncated"), "output: {output}");
}

#[test]
fn render_json_is_object_rooted_and_contains_agent_fields() {
    let result = fixture_result("json body", false, false);

    let payload: JsonValue = serde_json::from_str(&render_json(&result).unwrap()).unwrap();
    let leaf = payload.get("leaf").expect("missing leaf object");

    assert_eq!(leaf["title"], "Rendered");
    assert_eq!(leaf["file"], "rendered.md");
    assert_eq!(leaf["path"], "/tmp/rendered.md");
    assert_eq!(leaf["url"], "https://example.com/rendered");
    assert_eq!(leaf["frontmatter"]["title"], "Rendered");
    assert_eq!(leaf["frontmatter_raw"], "---\ntitle: Rendered\n---\n");
    assert_eq!(leaf["body"], "json body");
    assert_eq!(leaf["truncated"], false);
    assert_eq!(leaf["full"], false);
}

fn fixture_result(body: &str, truncated: bool, full: bool) -> ShowResult {
    let mut frontmatter = Mapping::new();
    frontmatter.insert(
        Value::String("title".to_string()),
        Value::String("Rendered".to_string()),
    );
    frontmatter.insert(
        Value::String("url".to_string()),
        Value::String("https://example.com/rendered".to_string()),
    );

    ShowResult {
        title: "Rendered".to_string(),
        file: "rendered.md".to_string(),
        path: "/tmp/rendered.md".to_string(),
        url: Some("https://example.com/rendered".to_string()),
        frontmatter,
        frontmatter_raw: "---\ntitle: Rendered\n---\n".to_string(),
        body: body.to_string(),
        truncated,
        full,
    }
}

fn write_index(tree: &Path, entries: &[(&str, &str)]) {
    fs::create_dir_all(tree).unwrap();
    let content = entries
        .iter()
        .map(|(file, title)| {
            serde_json::to_string(&index::IndexEntry {
                file: (*file).to_string(),
                title: (*title).to_string(),
                url: format!("https://example.com/{}", file.trim_end_matches(".md")),
            })
            .unwrap()
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(tree.join("index.jsonl"), format!("{content}\n")).unwrap();
}

fn write_leaf(tree: &Path, file: &str, frontmatter_fields: &str, body: &str) {
    write_raw_file(
        tree,
        file,
        &format!("---\n{frontmatter_fields}---\n\n{body}"),
    );
}

fn write_raw_file(tree: &Path, file: &str, content: &str) {
    let path = tree.join(file);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, content).unwrap();
}

fn snapshot_tree(root: &Path) -> BTreeMap<String, FileSnapshot> {
    let mut snapshot = BTreeMap::new();
    collect_snapshots(root, root, &mut snapshot);
    snapshot
}

fn collect_snapshots(root: &Path, dir: &Path, snapshot: &mut BTreeMap<String, FileSnapshot>) {
    let mut entries = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            collect_snapshots(root, &path, snapshot);
        } else {
            let metadata = fs::metadata(&path).unwrap();
            let key = path.strip_prefix(root).unwrap().display().to_string();
            snapshot.insert(
                key,
                FileSnapshot {
                    len: metadata.len(),
                    modified: metadata.modified().ok(),
                    contents: fs::read_to_string(&path).unwrap(),
                },
            );
        }
    }
}
