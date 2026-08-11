mod common;

use common::{command, TempHome};
use std::path::Path;
use std::process::Output;

fn run(home: &Path, key: Option<&str>, args: &[&str]) -> Output {
    let mut command = command(home);
    command.args(args);
    match key {
        Some(key) => command.env("BO_API_KEY", key),
        None => command.env_remove("BO_API_KEY"),
    };
    command.output().unwrap()
}

#[test]
fn agent_detects_a_non_empty_api_key() {
    let home = TempHome::new("agent-key", true);
    let output = run(home.path(), Some("test-key"), &["agent", "notes"]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(!output.status.success());
    assert!(stderr.contains("no raw Markdown documents"));
    assert!(!stderr.contains("BO_API_KEY"));
}

#[test]
fn agent_rejects_an_empty_api_key() {
    let home = TempHome::new("agent-empty-key", true);
    let output = run(home.path(), Some(""), &["agent", "notes"]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("BO_API_KEY is not set"));
}

#[test]
fn agent_limit_flags_are_wired_to_cli() {
    let home = TempHome::new("agent-flags", true);
    let output = run(
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
    assert!(String::from_utf8_lossy(&output.stderr).contains("no raw Markdown documents"));
}
