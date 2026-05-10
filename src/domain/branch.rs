// Branch file I/O.
//
// A branch is a synthesised concept file written by `bo compile`.
// It lives at {output_dir}/branches/{slug}.md and has YAML frontmatter
// followed by a markdown body beginning with a heading matching the title.

use crate::domain::frontmatter;
use serde_yaml_ng::{Mapping, Value};
use std::fs;
use std::io;
use std::path::Path;

/// Read the `compiled_at` value from an existing branch file.
///
/// Returns `None` in all failure cases: file absent, I/O error, unparseable
/// frontmatter, or missing `compiled_at` field.  The caller treats all of
/// these identically (first-write semantics).
pub fn read_compiled_at(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let (mapping, _) = frontmatter::parse(&content).ok()?;
    mapping
        .get("compiled_at")
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

/// Write a complete branch markdown file.
///
/// If `body` does not already begin with `# {title}`, the heading is
/// prepended automatically so the file always starts with the correct heading.
///
/// Parent directories are created as needed.
pub fn write(
    path: &Path,
    title: &str,
    body: &str,
    leaves: &[String],
    compiled_at: &str,
    updated_at: &str,
) -> io::Result<()> {
    // Build frontmatter mapping
    let mut mapping = Mapping::new();
    frontmatter::set_field(&mut mapping, "title", Value::String(title.to_string()));
    frontmatter::set_field(
        &mut mapping,
        "compiled_at",
        Value::String(compiled_at.to_string()),
    );
    frontmatter::set_field(
        &mut mapping,
        "updated_at",
        Value::String(updated_at.to_string()),
    );

    let leaves_seq = Value::Sequence(leaves.iter().map(|l| Value::String(l.clone())).collect());
    frontmatter::set_field(&mut mapping, "leaves", leaves_seq);

    // Ensure body starts with the correct heading
    let expected_heading = format!("# {}", title);
    let full_body = if body.starts_with(&expected_heading) {
        body.to_string()
    } else {
        format!("{}\n\n{}", expected_heading, body)
    };

    let content = frontmatter::render(&mapping, &full_body);

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

#[cfg(test)]
#[path = "../tests/domain_branch_tests.rs"]
mod tests;
