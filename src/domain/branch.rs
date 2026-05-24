// Branch file I/O.
//
// A branch is a synthesised concept file written by `bo compile`.
// It lives at {output_dir}/branches/{slug}.md and has YAML frontmatter
// followed by a markdown body beginning with a heading matching the title.

use crate::domain::frontmatter;
use serde_yaml_ng::{Mapping, Value};

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
        .expect("yaml serialization failure in branch frontmatter")
}

#[cfg(test)]
#[path = "../tests/domain_branch_tests.rs"]
mod tests;
