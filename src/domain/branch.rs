// Branch file I/O.
//
// A branch is a synthesised concept file written by `bo compile`.
// It lives at {output_dir}/branches/{slug}.md and has YAML frontmatter
// followed by a markdown body beginning with a heading matching the title.

use crate::domain::frontmatter;
use crate::domain::{Slug, Timestamp, Title};
use serde_yaml_ng::{Mapping, Value};
use std::fs;
use std::io;
use std::path::Path;

/// Read the `created_at` value from an existing branch file.
///
/// Returns `None` in all failure cases: file absent, I/O error, unparseable
/// frontmatter, or missing `created_at` field.  The caller treats all of
/// these identically (first-write semantics).
pub fn read_created_at(path: &Path) -> Option<Timestamp> {
    let content = fs::read_to_string(path).ok()?;
    let (mapping, _) = frontmatter::parse(&content).ok()?;
    mapping
        .get("created_at")
        .and_then(|v| v.as_str())
        .and_then(|s| Timestamp::parse(s).ok())
}

/// Write a complete branch markdown file.
///
/// If `body` does not already begin with `# {title}`, the heading is
/// prepended automatically so the file always starts with the correct heading.
///
/// Parent directories are created as needed.
pub fn write(
    path: &Path,
    title: &Title,
    body: &str,
    leaves: &[Slug],
    created_at: &Timestamp,
    updated_at: &Timestamp,
) -> io::Result<()> {
    let leaf_strs: Vec<String> = leaves.iter().map(|s| s.to_string()).collect();
    let content = format_content(
        title.as_str(),
        body,
        &leaf_strs,
        &created_at.to_rfc3339_millis(),
        &updated_at.to_rfc3339_millis(),
    );

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)
}

pub(crate) fn format_content(
    title: &str,
    body: &str,
    leaves: &[String],
    created_at: &str,
    updated_at: &str,
) -> String {
    // Build frontmatter mapping
    let mut mapping = Mapping::new();
    frontmatter::set_field(&mut mapping, "title", Value::String(title.to_string()));
    frontmatter::set_field(
        &mut mapping,
        "created_at",
        Value::String(created_at.to_string()),
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

    frontmatter::render(&mapping, &full_body)
}

#[cfg(test)]
#[path = "../tests/domain_branch_tests.rs"]
mod tests;
