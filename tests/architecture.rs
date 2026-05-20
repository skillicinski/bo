//! Enforces layer direction: domain ← engine ← adapters ← cli.
//!
//! These dependencies are unidirectional. Adding `use crate::cli` in
//! `src/engine/foo.rs` should fail this test.

use std::fs;
use std::path::{Path, PathBuf};

#[test]
fn domain_does_not_depend_on_upper_layers() {
    assert_no_imports(
        "src/domain",
        &["crate::engine", "crate::adapters", "crate::cli"],
    );
}

#[test]
fn engine_does_not_depend_on_cli_or_adapters() {
    assert_no_imports("src/engine", &["crate::cli", "crate::adapters"]);
}

#[test]
fn adapters_does_not_depend_on_cli() {
    assert_no_imports("src/adapters", &["crate::cli"]);
}

fn assert_no_imports(dir: &str, forbidden: &[&str]) {
    let mut violations = Vec::new();
    walk(Path::new(dir), &mut |path: &Path| {
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            return;
        }
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return,
        };
        for (line_no, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("use ") {
                continue;
            }
            for fb in forbidden {
                if trimmed.contains(fb) {
                    violations.push(format!(
                        "{}:{}: forbidden import `{}`: {}",
                        path.display(),
                        line_no + 1,
                        fb,
                        line.trim()
                    ));
                }
            }
        }
    });
    assert!(
        violations.is_empty(),
        "layer-direction violations:\n{}",
        violations.join("\n")
    );
}

fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path: PathBuf = entry.path();
        if path.is_dir() {
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
}
