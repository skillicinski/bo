// ── compile prompt construction ───────────────────────────────────────────────

use std::collections::{HashMap, HashSet};
use std::fs;

use crate::domain::frontmatter;
use crate::domain::manifest::Manifest;
use crate::engine::config::SeededConfig;

use super::plan::LoadedLeaf;

pub(super) const COMPILE_SYSTEM_PROMPT: &str = "\
You are a knowledge compilation engine for a personal document collection.

Your task: identify recurring concepts and themes that appear across multiple \
documents, then produce structured output describing each concept.

## Rules

- A concept MUST appear in at least two documents. Never create a branch with only one leaf. \
  If a topic only appears in a single document, do not create a branch for it — it is not a \
  cross-cutting concept.
- Prefer specific, recurring themes over broad catch-all categories.
- Each branch body should synthesise how the concept manifests across the documents — \
  draw connections, note contrasts, highlight patterns. Do not just summarise each document \
  in turn.
- The body should begin with a single markdown heading matching the title (e.g. `# Concept Name`). \
  Do not repeat the heading or nest a second heading immediately after.
- Reference documents by their filename only when making a specific point about that document's \
  contribution to the concept.
- Only use document filenames exactly as provided in the input.
- If no cross-cutting concepts span two or more documents, return an empty branches array.
";

pub(super) fn build_user_message(leaves: &[LoadedLeaf]) -> String {
    let mut msg = format!(
        "Please compile my knowledge base. There are {} documents.\n\n",
        leaves.len()
    );

    for leaf in leaves {
        msg.push_str(&format!(
            "<document filename=\"{}\" title=\"{}\">\n{}\n</document>\n\n",
            leaf.filename, leaf.title, leaf.body
        ));
    }

    msg
}

pub(super) fn build_incremental_user_message(
    cfg: &SeededConfig,
    manifest: &Manifest,
    leaves: &[LoadedLeaf],
    new_leaf_slugs: &[String],
    stale_branch_slugs: &[String],
) -> String {
    let leaves_by_slug: HashMap<&str, &LoadedLeaf> = leaves
        .iter()
        .map(|leaf| (leaf.slug.as_str(), leaf))
        .collect();
    let new_leaf_slugs: HashSet<&str> = new_leaf_slugs.iter().map(String::as_str).collect();
    let stale_branch_slugs: HashSet<&str> = stale_branch_slugs.iter().map(String::as_str).collect();

    let mut msg = String::from(
        "Please incrementally compile my knowledge base. Preserve omitted non-stale branches.\n\n",
    );

    msg.push_str("<existing_branches>\n");
    for branch_record in &manifest.branches {
        let stale = stale_branch_slugs.contains(branch_record.slug.as_str());
        let leaves_str: Vec<&str> = branch_record.leaves.iter().map(|s| s.as_str()).collect();
        msg.push_str(&format!(
            "<branch slug=\"{}\" title=\"{}\" stale=\"{}\" leaves=\"{}\">\n",
            branch_record.slug,
            branch_record.title,
            stale,
            leaves_str.join(",")
        ));
        let branch_path = cfg.tree.output_dir.join(&branch_record.file);
        if let Ok(content) = fs::read_to_string(branch_path) {
            if let Ok((_, body)) = frontmatter::parse(&content) {
                msg.push_str("<branch_body>\n");
                msg.push_str(&body);
                msg.push_str("\n</branch_body>\n");
            }
        }
        msg.push_str("</branch>\n");
    }
    msg.push_str("</existing_branches>\n\n");

    msg.push_str("<leaf_catalogue>\n");
    for leaf in leaves {
        msg.push_str(&format!(
            "<leaf slug=\"{}\" file=\"{}\" title=\"{}\" collected_at=\"{}\">\n",
            leaf.slug, leaf.filename, leaf.title, leaf.collected_at
        ));
        if let Some(summary) = &leaf.summary {
            msg.push_str("<summary>");
            msg.push_str(summary);
            msg.push_str("</summary>\n");
        }
        msg.push_str("</leaf>\n");
    }
    msg.push_str("</leaf_catalogue>\n\n");

    let mut full_body_slugs: HashSet<&str> = new_leaf_slugs.clone();
    for branch_record in &manifest.branches {
        if stale_branch_slugs.contains(branch_record.slug.as_str()) {
            for leaf_slug in &branch_record.leaves {
                full_body_slugs.insert(leaf_slug.as_str());
            }
        }
    }

    msg.push_str("<full_leaf_bodies>\n");
    for slug in full_body_slugs {
        if let Some(leaf) = leaves_by_slug.get(slug) {
            msg.push_str(&format!(
                "<document slug=\"{}\" filename=\"{}\" title=\"{}\">\n{}\n</document>\n",
                leaf.slug, leaf.filename, leaf.title, leaf.body
            ));
        }
    }
    msg.push_str("</full_leaf_bodies>\n");

    msg
}
