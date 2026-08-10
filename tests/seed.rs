use std::ffi::OsStr;
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
            "bo-seed-integration-{label}-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
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

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bo"))
        .args(args)
        .env("HOME", home)
        .env_remove("USERPROFILE")
        .output()
        .unwrap()
}

fn assert_success(output: &Output, expected_path: &Path) {
    assert!(
        output.status.success(),
        "expected success, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("seeded at {}\n", expected_path.display())
    );
    assert!(output.stderr.is_empty());
}

fn assert_failure(output: &Output) {
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("seeding failed: "));
}

#[test]
fn seed_creates_random_adjective_noun_directory() {
    let home = TempHome::new("random");
    let output = run(home.path(), &["seed"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let path = PathBuf::from(stdout.trim_start_matches("seeded at ").trim());

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert!(stdout.starts_with("seeded at "));
    assert!(path.starts_with(home.path().join(".bo")));

    let name = path.file_name().and_then(OsStr::to_str).unwrap();
    assert!(name
        .chars()
        .all(|character| character.is_ascii_lowercase() || character == '-'));
    assert_eq!(name.matches('-').count(), 1);
    assert!(path.is_dir());
    assert_eq!(
        fs::read_to_string(path.join("state.json")).unwrap(),
        "{\n  \"raw\": [],\n  \"summaries\": []\n}\n"
    );
}

#[test]
fn seed_with_name_creates_named_directory() {
    let home = TempHome::new("named");
    let expected_path = home.path().join(".bo/test");
    let output = run(home.path(), &["seed", "--name", "test"]);

    assert_success(&output, &expected_path);
    assert!(expected_path.is_dir());
    assert_eq!(
        fs::read_to_string(expected_path.join("state.json")).unwrap(),
        "{\n  \"raw\": [],\n  \"summaries\": []\n}\n"
    );
}

#[test]
fn seed_fails_when_default_location_cannot_be_created() {
    let home = TempHome::new("blocked");
    fs::write(home.path().join(".bo"), "not a directory").unwrap();

    let output = run(home.path(), &["seed"]);

    assert_failure(&output);
}

#[test]
fn seed_fails_on_existing_directory_without_modifying_it() {
    let home = TempHome::new("existing");
    let path = home.path().join(".bo/test");
    fs::create_dir_all(&path).unwrap();
    fs::write(path.join("keep"), "content").unwrap();

    let output = run(home.path(), &["seed", "--name", "test"]);

    assert_failure(&output);
    assert_eq!(fs::read_to_string(path.join("keep")).unwrap(), "content");
}

#[test]
fn seed_rejects_names_that_cannot_be_directory_components() {
    let home = TempHome::new("invalid");
    let invalid_names = [
        "",
        ".",
        "..",
        "../escape",
        "sub/name",
        "sub\\name",
        "CON",
        "a:b",
    ];

    for name in invalid_names {
        let output = run(home.path(), &["seed", "--name", name]);
        assert_failure(&output);
    }

    assert!(!home.path().join("escape").exists());
    assert!(!home.path().join(".bo").exists());
}
