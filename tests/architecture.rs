//! Enforces the inward dependency order in `docs/architecture.md`:
//! `main → cli → adapters → engine → domain`.
//!
//! Layers may skip intermediate layers; references must never point left.

use std::fs;
use std::path::Path;

const LAYER_DIRS: &[&str] = &["src/domain", "src/engine", "src/adapters", "src/cli"];

#[test]
fn domain_does_not_depend_on_upper_layers() {
    assert_no_references(
        "src/domain",
        &[
            "crate::engine",
            "crate::adapters",
            "crate::cli",
            "super::engine",
            "super::adapters",
            "super::cli",
        ],
    );
}

#[test]
fn engine_does_not_depend_on_cli_or_adapters() {
    assert_no_references(
        "src/engine",
        &[
            "crate::cli",
            "crate::adapters",
            "super::cli",
            "super::adapters",
        ],
    );
}

#[test]
fn adapters_does_not_depend_on_cli() {
    assert_no_references("src/adapters", &["crate::cli", "super::cli"]);
}

#[test]
fn cross_layer_paths_are_scannable() {
    for dir in LAYER_DIRS {
        assert_no_references(
            dir,
            &[
                "crate::{",
                "super::super::",
                "use crate as ",
                "extern crate self as ",
            ],
        );
    }
}

#[test]
#[should_panic(expected = "forbidden architecture reference `crate::cli`")]
fn fully_qualified_forbidden_references_fail() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("forbidden.rs"),
        "pub fn violate() { crate::cli::run(); }\n",
    )
    .unwrap();
    assert_no_references(dir.path().to_str().unwrap(), &["crate::cli"]);
}

fn assert_no_references(dir: &str, forbidden: &[&str]) {
    let mut violations = Vec::new();
    walk(Path::new(dir), &mut |path: &Path| {
        if path.extension().and_then(|s| s.to_str()) != Some("rs") {
            return;
        }
        let content = fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        for (line_no, line) in content.lines().enumerate() {
            for reference in forbidden {
                if line.contains(reference) {
                    violations.push(format!(
                        "{}:{}: forbidden architecture reference `{}`: {}",
                        path.display(),
                        line_no + 1,
                        reference,
                        line.trim()
                    ));
                }
            }
        }
    });
    assert!(
        violations.is_empty(),
        "architecture violations:\n{}",
        violations.join("\n")
    );
}

fn walk(dir: &Path, visit: &mut dyn FnMut(&Path)) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("failed to read {} entry: {error}", dir.display()))
            .path();
        if path.is_dir() {
            walk(&path, visit);
        } else {
            visit(&path);
        }
    }
}
