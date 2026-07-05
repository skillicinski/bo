// Domain entity and I/O for branch documents.
//
// A branch is a synthesised concept file written by `bo compile`.
// It lives at {output_dir}/branches/{slug}.md and has YAML frontmatter
// followed by a markdown body beginning with a heading matching the title.

use crate::domain::{Slug, Timestamp, Title};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

// ── Branch ────────────────────────────────────────────────────────────────────

/// A synthesised concept in the knowledge graph, grouping related leaves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Branch {
    pub slug: Slug,
    pub file: String,
    pub title: Title,
    /// First compile run that produced this branch. Preserved across recompiles.
    pub created_at: Timestamp,
    /// Most recent compile run that touched this branch. Updated every recompile.
    pub updated_at: Timestamp,
    /// Slugs of leaves assigned to this branch. Canonical direction of the
    /// cross-reference.
    pub leaves: Vec<Slug>,
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
    mapping.insert(
        Value::String("title".to_string()),
        Value::String(title.to_string()),
    );
    mapping.insert(
        Value::String("created_at".to_string()),
        Value::String(created_at.to_string()),
    );
    mapping.insert(
        Value::String("updated_at".to_string()),
        Value::String(updated_at.to_string()),
    );

    let leaves_seq = Value::Sequence(leaves.iter().map(|l| Value::String(l.clone())).collect());
    mapping.insert(Value::String("leaves".to_string()), leaves_seq);

    // Ensure body starts with the correct heading
    let expected_heading = format!("# {}", title);
    let full_body = if body.starts_with(&expected_heading) {
        body.to_string()
    } else {
        format!("{}\n\n{}", expected_heading, body)
    };

    crate::domain::frontmatter::render(&mapping, &full_body)
        .expect("yaml serialization failure in branch frontmatter")
}

#[cfg(test)]
#[path = "../tests/domain_branch_tests.rs"]
mod tests;
