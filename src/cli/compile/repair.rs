// ── stale branch repair: classify → prune → repair → assemble ─────────────

use std::collections::HashSet;
use std::fs;

use serde_yaml_ng::Value;

use crate::domain::frontmatter;
use crate::domain::manifest::Manifest;
use crate::domain::slug::Slug;
use crate::domain::Leaf;
use crate::engine::config::SeededConfig;

use super::plan::select_new_leaf_slugs;
use super::CompileError;

// ── types ─────────────────────────────────────────────────────────────────────

/// Classification of leaf files for the planning phase.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct LeafFileClassification {
    pub(super) deleted_leaf_slugs: Vec<String>,
    pub(super) skipped_leaf_slugs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RemovedBranchResult {
    pub(super) slug: String,
    pub(super) title: String,
    pub(super) remaining_leaf_count: usize,
    pub(super) reason: String,
}

// ── functions ─────────────────────────────────────────────────────────────────

pub(super) fn classify_leaf_files(
    cfg: &SeededConfig,
    manifest: &Manifest,
    new_leaf_slugs: &[String],
) -> Result<LeafFileClassification, CompileError> {
    let tree = cfg.tree();
    let new_leaf_slugs: HashSet<&str> = new_leaf_slugs.iter().map(String::as_str).collect();

    let mut deleted_leaf_slugs = Vec::new();
    let mut skipped_leaf_slugs = Vec::new();

    for leaf in &manifest.leaves {
        let leaf_path = tree.join(&leaf.file);
        let is_new = new_leaf_slugs.contains(leaf.slug.as_str());

        match fs::read_to_string(&leaf_path) {
            Ok(content) => {
                if frontmatter::parse(&content).is_err() {
                    if is_new {
                        return Err(CompileError::Io(format!(
                            "newly selected leaf '{}' is malformed; no files were changed",
                            leaf.file
                        )));
                    }
                    skipped_leaf_slugs.push(leaf.slug.as_str().to_string());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Missing file: always add to deleted_leaf_slugs.
                // Unbranched leaves are pruned; branch-referenced leaves are
                // repaired (removed from branch, branch dropped if below minimum).
                deleted_leaf_slugs.push(leaf.slug.as_str().to_string());
            }
            Err(error) => {
                if is_new {
                    return Err(CompileError::Io(format!(
                        "newly selected leaf '{}' is unreadable: {}; no files were changed",
                        leaf.file, error
                    )));
                }
                skipped_leaf_slugs.push(leaf.slug.as_str().to_string());
            }
        }
    }

    Ok(LeafFileClassification {
        deleted_leaf_slugs,
        skipped_leaf_slugs,
    })
}

/// Deterministic pre-pass: detect deleted leaves, remove them from branches,
/// drop branches below 2-leaf minimum, purge orphan leaf records.
/// Writes the repaired manifest if changes were made.
pub(super) fn repair_stale_branches(
    cfg: &SeededConfig,
    manifest: &Manifest,
) -> Result<Vec<String>, CompileError> {
    let new_leaf_slugs = select_new_leaf_slugs(manifest)?;
    let classification = classify_leaf_files(cfg, manifest, &new_leaf_slugs)?;

    if classification.deleted_leaf_slugs.is_empty() {
        return Ok(Vec::new());
    }

    let deleted_set: HashSet<&str> = classification
        .deleted_leaf_slugs
        .iter()
        .map(String::as_str)
        .collect();

    let deleted_filenames: HashSet<&str> = manifest
        .leaves
        .iter()
        .filter(|l| deleted_set.contains(l.slug.as_str()))
        .map(|l| l.file.as_str())
        .collect();

    let branch_referenced_slugs: HashSet<&str> = manifest
        .branches
        .iter()
        .flat_map(|b| b.leaves.iter().map(|s| s.as_str()))
        .collect();
    let orphan_slugs: Vec<String> = classification
        .deleted_leaf_slugs
        .iter()
        .filter(|s| !branch_referenced_slugs.contains(s.as_str()))
        .cloned()
        .collect();

    let mut notifications = if !orphan_slugs.is_empty() {
        let n = orphan_slugs.len();
        let msg = format!(
            "pruned {} orphan leaf record{} (file{} missing, not in any branch)",
            n,
            if n == 1 { "" } else { "s" },
            if n == 1 { "" } else { "s" }
        );
        eprintln!("{}", msg);
        vec![msg]
    } else {
        Vec::new()
    };

    let mut branches_removed = Vec::new();
    let mut branch_deletes = Vec::new();
    let mut repaired_branches = Vec::new();
    let mut repaired_branch_slugs = Vec::new();

    for branch in &manifest.branches {
        let remaining: Vec<Slug> = branch
            .leaves
            .iter()
            .filter(|s| !deleted_set.contains(s.as_str()))
            .cloned()
            .collect();

        if remaining.len() < 2 {
            branch_deletes.push(branch.file.clone());
            branches_removed.push(RemovedBranchResult {
                slug: branch.slug.as_str().to_string(),
                title: branch.title.as_str().to_string(),
                remaining_leaf_count: remaining.len(),
                reason: "stale_branch_below_minimum_leaves".to_string(),
            });
        } else {
            let removed_count = branch.leaves.len() - remaining.len();
            if removed_count > 0 {
                repaired_branch_slugs.push(branch.slug.as_str().to_string());
                // Repair branch .md frontmatter: drop deleted leaf filenames
                // from leaves: list so the file matches the repaired manifest.
                // Body is left as-is (may reference removed leaves; --all
                // resynthesizes).
                let branch_path = cfg.tree().join(&branch.file);
                if let Ok(content) = fs::read_to_string(&branch_path) {
                    if let Ok((mut mapping, body)) = frontmatter::parse(&content) {
                        let leaves_key = Value::String("leaves".to_string());
                        if let Some(Value::Sequence(seq)) = mapping.get(&leaves_key) {
                            let filtered: Vec<Value> = seq
                                .iter()
                                .filter(|v| {
                                    if let Value::String(filename) = v {
                                        !deleted_filenames.contains(filename.as_str())
                                    } else {
                                        true
                                    }
                                })
                                .cloned()
                                .collect();
                            mapping.insert(leaves_key, Value::Sequence(filtered));
                        }
                        if let Ok(new_content) = frontmatter::render(&mapping, &body) {
                            let _ = fs::write(&branch_path, &new_content);
                            let note = format!(
                                "branch '{}' frontmatter repaired (body may reference removed leaves; recompile with --all to resynthesize)",
                                branch.slug.as_str()
                            );
                            notifications.push(note);
                        }
                    }
                }
            }
            let mut repaired = branch.clone();
            repaired.leaves = remaining;
            repaired_branches.push(repaired);
        }
    }

    // Emit messages for branch-level repairs (separate from orphan leaf pruning above).
    if !repaired_branch_slugs.is_empty() {
        let names = repaired_branch_slugs.join(", ");
        let msg = format!(
            "repaired {} branch{} with deleted leaves: {}",
            repaired_branch_slugs.len(),
            if repaired_branch_slugs.len() == 1 {
                ""
            } else {
                "es"
            },
            names,
        );
        eprintln!("{}", msg);
        notifications.push(msg);
    }
    if !branches_removed.is_empty() {
        let names: Vec<&str> = branches_removed.iter().map(|b| b.slug.as_str()).collect();
        let msg = format!(
            "removed {} stale branch{} below threshold: {}",
            branches_removed.len(),
            if branches_removed.len() == 1 {
                ""
            } else {
                "es"
            },
            names.join(", "),
        );
        eprintln!("{}", msg);
        notifications.push(msg);
    }

    let repaired_leaves: Vec<Leaf> = manifest
        .leaves
        .iter()
        .filter(|l| !deleted_set.contains(l.slug.as_str()))
        .cloned()
        .collect();

    let repaired_manifest = Manifest {
        tree: manifest.tree.clone(),
        leaves: repaired_leaves,
        branches: repaired_branches,
    };

    // Write repaired manifest
    let tree = cfg.tree();
    let manifest_path = crate::domain::tree::manifest_path(tree.path());
    crate::engine::manifest::write(&manifest_path, &repaired_manifest)
        .map_err(|e| CompileError::Io(format!("failed to write repaired manifest: {}", e)))?;

    // Delete branch files
    for file in &branch_deletes {
        let path = tree.join(file);
        if path.exists() {
            std::fs::remove_file(&path).ok();
        }
    }

    Ok(notifications)
}
