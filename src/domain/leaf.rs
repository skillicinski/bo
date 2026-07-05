// Domain entity and I/O for leaf documents.
//
// A leaf is a collected document produced by `bo add`. It lives at
// {output_dir}/{slug}.md and has YAML frontmatter followed by a markdown body.
//
// Analogous to branch.rs; together they define the two entity types in bo's
// knowledge graph.
//
// Frontmatter is serialized via serde_yaml_ng, mirroring branch.rs. The
// leaf→branch relationship is not written into leaf frontmatter; the manifest
// is the sole source of truth (see `Manifest::branches_for_leaf`). Likewise,
// `summary` lives only in the manifest — it is never written to leaf
// frontmatter.

use crate::domain::title::deserialize_option_title;
use crate::domain::{Slug, Timestamp, Title, Url};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::{Mapping, Value};

// ── Leaf ──────────────────────────────────────────────────────────────────────

/// A collected document in the knowledge graph.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Leaf {
    pub slug: Slug,
    pub file: String,
    #[serde(default, deserialize_with = "deserialize_option_title")]
    pub title: Option<Title>,
    pub url: Url,
    pub collected_at: Timestamp,
    #[serde(default)]
    pub summary: Option<String>,
}

/// Format leaf document content — frontmatter block followed by body.
///
/// Only `title`, `url`, and `collected_at` are written to frontmatter.
/// `summary` and `updated_at` are deliberately omitted — the manifest is the
/// single source of truth for both.
pub fn format_content(
    title: Option<&Title>,
    url: &Url,
    collected_at: &Timestamp,
    body: &str,
) -> String {
    let mut mapping = Mapping::new();
    mapping.insert(
        Value::String("title".to_string()),
        Value::String(title.map(|t| t.as_str().to_string()).unwrap_or_default()),
    );
    mapping.insert(
        Value::String("url".to_string()),
        Value::String(url.as_str().to_string()),
    );
    mapping.insert(
        Value::String("collected_at".to_string()),
        Value::String(collected_at.to_rfc3339_millis()),
    );

    let mut full_body = String::new();
    if let Some(t) = title {
        full_body.push_str(&format!("# {}\n\n", t.as_str()));
    }
    full_body.push_str(body);
    if !full_body.ends_with('\n') {
        full_body.push('\n');
    }

    crate::domain::frontmatter::render(&mapping, &full_body)
        .expect("yaml serialization failure in leaf frontmatter")
}

#[cfg(test)]
#[path = "../tests/domain_leaf_tests.rs"]
mod tests;
