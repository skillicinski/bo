// Domain entity and I/O for branch documents.
//
// A branch is a synthesised concept file written by `bo synthesize`.
// It lives at {output_dir}/branch/{slug}.md and has YAML frontmatter
// followed by a markdown body beginning with a heading matching the title.

use crate::domain::{Slug, Timestamp, Title};
use serde::{Deserialize, Serialize};

// ── Branch ────────────────────────────────────────────────────────────────────

/// A synthesised concept in the knowledge graph, grouping related leaves.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Branch {
    pub slug: Slug,
    pub file: String,
    pub title: Title,
    /// First synthesis run that produced this branch. Preserved across re-synthesis.
    pub created_at: Timestamp,
    /// Most recent synthesis run that touched this branch. Updated every re-synthesis.
    pub updated_at: Timestamp,
    /// Slugs of leaves assigned to this branch. Canonical direction of the
    /// cross-reference.
    pub leaves: Vec<Slug>,
}

// ── typed frontmatter ────────────────────────────────────────────────────────

/// Frontmatter written to branch .md files. Field order matches the on-disk YAML
/// key order (title, created_at, updated_at, leaves).
///
/// `leaves` holds leaf filenames (with `.md` extension) as written to disk.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct BranchFrontmatter {
    pub title: Title,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub leaves: Vec<String>,
}

pub(crate) fn format_content(
    title: &Title,
    body: &str,
    leaves: &[String],
    created_at: &Timestamp,
    updated_at: &Timestamp,
) -> String {
    let fm = BranchFrontmatter {
        title: title.clone(),
        created_at: created_at.clone(),
        updated_at: updated_at.clone(),
        leaves: leaves.to_vec(),
    };

    // Ensure body starts with the correct heading
    let expected_heading = format!("# {}", title.as_str());
    let full_body = if body.starts_with(&expected_heading) {
        body.to_string()
    } else {
        format!("{}\n\n{}", expected_heading, body)
    };

    crate::domain::frontmatter::render(&fm, &full_body)
        .expect("yaml serialization failure in branch frontmatter")
}

#[cfg(test)]
#[path = "../tests/domain_branch_tests.rs"]
mod tests;
