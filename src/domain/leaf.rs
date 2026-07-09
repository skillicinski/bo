// Domain entity and I/O for leaf documents.
//
// A leaf is a collected document produced by `bo add`. It lives at
// {output_dir}/leaf/{slug}.md and has YAML frontmatter followed by a markdown body.
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

// ── typed frontmatter ────────────────────────────────────────────────────────

/// Frontmatter written to leaf .md files. Field order matches the on-disk YAML key
/// order (title, url, collected_at).
///
/// `title` is String (not Option<Title>) because the on-disk format writes
/// `title: ''` for missing titles — null/absent would change the format.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct LeafFrontmatter {
    pub title: String,
    pub url: Url,
    pub collected_at: Timestamp,
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
    let fm = LeafFrontmatter {
        title: title.map(|t| t.as_str().to_string()).unwrap_or_default(),
        url: url.clone(),
        collected_at: collected_at.clone(),
    };

    let mut full_body = String::new();
    if let Some(t) = title {
        // Don't duplicate the title when the extracted body already opens with
        // an H1 matching it — e.g. an article that leads with a byline before
        // its own `# Title`. The prepend exists for bodies that lost their
        // heading during extraction.
        if !body_has_title_h1(body, t.as_str()) {
            full_body.push_str(&format!("# {}\n\n", t.as_str()));
        }
    }
    full_body.push_str(body);
    if !full_body.ends_with('\n') {
        full_body.push('\n');
    }

    crate::domain::frontmatter::render(&fm, &full_body)
        .expect("yaml serialization failure in leaf frontmatter")
}

/// True if `body`'s first H1 matches `title` (case-insensitive, trimmed), so
/// `format_content` should not prepend its own heading. Only the first `# `
/// line is considered — a heading deeper in the body is a section, not the
/// title. The H1 need not be the very first line: articles often lead with a
/// byline before the heading (issue #161).
fn body_has_title_h1(body: &str, title: &str) -> bool {
    let want = title.trim().to_lowercase();
    if want.is_empty() {
        return false;
    }
    for line in body.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("# ") {
            return rest.trim().to_lowercase() == want;
        }
    }
    false
}

#[cfg(test)]
#[path = "../tests/domain_leaf_tests.rs"]
mod tests;
