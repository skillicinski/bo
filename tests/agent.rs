use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct TempHome(PathBuf);

impl TempHome {
    fn new(label: &str) -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "bo-agent-integration-{label}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(path.join(".bo/notes")).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn run(home: &Path, key: Option<&str>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bo"));
    command
        .args(args)
        .env("HOME", home)
        .env_remove("USERPROFILE")
        .env_remove("BO_API_URL");
    match key {
        Some(key) => command.env("BO_API_KEY", key),
        None => command.env_remove("BO_API_KEY"),
    };
    command.output().unwrap()
}

#[test]
fn agent_detects_a_non_empty_api_key() {
    let home = TempHome::new("key");
    let output = run(home.path(), Some("test-key"), &["agent", "notes"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("no raw Markdown documents"));
    assert!(!stderr.contains("BO_API_KEY"));
}

#[test]
fn agent_rejects_an_empty_api_key() {
    let home = TempHome::new("empty-key");
    let output = run(home.path(), Some(""), &["agent", "notes"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("BO_API_KEY is not set"));
}

#[test]
fn agent_flags_parse_and_report_invalid_values() {
    let home = TempHome::new("flags");
    let valid = run(
        home.path(),
        Some("test-key"),
        &[
            "agent",
            "notes",
            "--max-turns",
            "1",
            "--max-tool-calls",
            "2",
            "--max-tool-output-bytes",
            "3",
            "--max-response-tokens",
            "4",
            "--timeout-seconds",
            "5",
        ],
    );
    assert!(String::from_utf8_lossy(&valid.stderr).contains("no raw Markdown documents"));
    assert!(!String::from_utf8_lossy(&valid.stderr).contains("unknown"));

    let unsupported = run(
        home.path(),
        None,
        &["agent", "notes", "--max-turns", "not-a-number"],
    );
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("positive integer"));

    let missing = run(home.path(), None, &["agent", "notes", "--timeout-seconds"]);
    assert!(String::from_utf8_lossy(&missing.stderr).contains("missing value"));

    let unknown = run(
        home.path(),
        None,
        &["agent", "notes", "--not-an-option", "1"],
    );
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("usage: bo agent"));
}
