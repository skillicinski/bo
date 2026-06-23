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
) -> String {
    let tree = cfg.tree();
    let leaves_by_slug: HashMap<&str, &LoadedLeaf> = leaves
        .iter()
        .map(|leaf| (leaf.slug.as_str(), leaf))
        .collect();
    let new_leaf_slugs: HashSet<&str> = new_leaf_slugs.iter().map(String::as_str).collect();

    let mut msg = String::from(
        "Please incrementally compile my knowledge base.\n\n\
         INCREMENTAL RULES:\n\
         - You are integrating NEW leaves (listed in <full_leaf_bodies>) into an existing branch structure.\n\
         - Use `updated_branches` to modify an existing branch — include its slug, updated body, and full leaf list (existing + new).\n\
         - Use `new_branches` only for entirely new concepts not covered by any existing branch. Each new branch must include at least one new leaf.\n\
         - Do NOT place an existing branch in `new_branches` — that causes a duplicate slug error.\n\
         - Do NOT output branches that only contain previously-compiled leaves with no new leaf added.\n\
         - Omit unchanged branches entirely — they are preserved automatically.\n\
         - If no new leaf fits any existing or new cross-cutting concept, return empty arrays.\n\n",
    );

    msg.push_str("<existing_branches>\n");
    for branch_record in &manifest.branches {
        let leaves_str: Vec<&str> = branch_record.leaves.iter().map(|s| s.as_str()).collect();
        msg.push_str(&format!(
            "<branch slug=\"{}\" title=\"{}\" leaves=\"{}\">\n",
            branch_record.slug,
            branch_record.title,
            leaves_str.join(",")
        ));
        let branch_path = tree.join(&branch_record.file);
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

    msg.push_str("<new_leaves_to_integrate>\n");
    for slug in &new_leaf_slugs {
        msg.push_str(&format!("  - {}\n", slug));
    }
    msg.push_str("</new_leaves_to_integrate>\n\n");
    msg.push_str("The above are the NEW leaves you must integrate. Every branch you output (updated or new) MUST include at least one of these new leaves. If none fit any concept, return empty arrays.\n\n");

    let full_body_slugs: HashSet<&str> = new_leaf_slugs.clone();

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
