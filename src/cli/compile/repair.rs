// ── stale branch repair: classify → prune → repair → assemble ─────────────

use std::collections::HashSet;
use std::fs;

use serde::Serialize;
use serde_yaml_ng::Value;

use crate::domain::frontmatter;
use crate::domain::manifest::Manifest;
use crate::domain::slug::Slug;
use crate::domain::{Branch, Leaf};
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct RemovedBranchResult {
    pub(super) slug: String,
    pub(super) title: String,
    pub(super) remaining_leaf_count: usize,
    pub(super) reason: String,
}

/// Computed deletion sets derived from leaf-file classification.
struct DeletionClassification {
    deleted_slugs: Vec<String>,
    deleted_set: HashSet<String>,
    deleted_filenames: HashSet<String>,
    orphan_slugs: Vec<String>,
}

/// Outcome of the branch-repair pass.
struct BranchRepairOutcome {
    repaired_branches: Vec<Branch>,
    branch_deletes: Vec<String>,
    repaired_branch_slugs: Vec<String>,
    branches_removed: Vec<RemovedBranchResult>,
    frontmatter_notes: Vec<String>,
}

/// Structured outcome of the repair pass, for journaling. `notifications`
/// carries the human-facing summary strings; the typed fields carry the
/// "what was pruned and why" detail.
#[derive(Debug, Clone, Default)]
pub(super) struct RepairReport {
    pub(super) notifications: Vec<String>,
    pub(super) orphan_leaf_slugs: Vec<String>,
    pub(super) repaired_branch_slugs: Vec<String>,
    pub(super) removed_branches: Vec<RemovedBranchResult>,
}

impl RepairReport {
    /// True when repair pruned or rewrote nothing.
    pub(super) fn is_empty(&self) -> bool {
        self.orphan_leaf_slugs.is_empty()
            && self.repaired_branch_slugs.is_empty()
            && self.removed_branches.is_empty()
    }
}

// ── public functions ──────────────────────────────────────────────────────────

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

/// Read-only check: are there deleted leaf files whose absence would require
/// stale-branch repair? Dry-run paths use this to fail with an actionable
/// diagnostic instead of writing the repaired manifest/branches.
pub(super) fn requires_repair(
    cfg: &SeededConfig,
    manifest: &Manifest,
) -> Result<bool, CompileError> {
    let class = classify_deletions(cfg, manifest)?;
    Ok(!class.deleted_slugs.is_empty())
}

/// Deterministic pre-pass: detect deleted leaves, remove them from branches,
/// drop branches below 2-leaf minimum, purge orphan leaf records.
/// Writes the repaired manifest if changes were made.
pub(super) fn repair_stale_branches(
    cfg: &SeededConfig,
    manifest: &Manifest,
) -> Result<RepairReport, CompileError> {
    let class = classify_deletions(cfg, manifest)?;
    if class.deleted_slugs.is_empty() {
        return Ok(RepairReport::default());
    }

    let mut notifications = Vec::new();
    if let Some(msg) = orphan_prune_message(&class.orphan_slugs) {
        notifications.push(msg);
    }

    let outcome = repair_branch_files(cfg, manifest, &class);
    notifications.extend(outcome.frontmatter_notes.iter().cloned());

    if let Some(msg) = repaired_summary_message(&outcome.repaired_branch_slugs) {
        notifications.push(msg);
    }
    if let Some(msg) = removed_summary_message(&outcome.branches_removed) {
        notifications.push(msg);
    }

    let repaired_manifest = assemble_repaired_manifest(manifest, &class, &outcome);

    // Write repaired manifest
    let tree = cfg.tree();
    let manifest_path = crate::domain::tree::manifest_path(tree.path());
    crate::engine::manifest::write(&manifest_path, &repaired_manifest)
        .map_err(|e| CompileError::Io(format!("failed to write repaired manifest: {}", e)))?;

    // Delete branch files
    for file in &outcome.branch_deletes {
        let path = tree.join(file);
        if path.exists() {
            std::fs::remove_file(&path).ok();
        }
    }

    Ok(RepairReport {
        notifications,
        orphan_leaf_slugs: class.orphan_slugs,
        repaired_branch_slugs: outcome.repaired_branch_slugs,
        removed_branches: outcome.branches_removed,
    })
}

#[cfg(test)]
#[path = "../../tests/cli_compile_repair_tests.rs"]
mod repair_tests;

// ── pipeline stages ───────────────────────────────────────────────────────────

/// Classify deleted leaves and compute the deletion sets needed by later stages.
fn classify_deletions(
    cfg: &SeededConfig,
    manifest: &Manifest,
) -> Result<DeletionClassification, CompileError> {
    let new_leaf_slugs = select_new_leaf_slugs(manifest)?;
    let classification = classify_leaf_files(cfg, manifest, &new_leaf_slugs)?;
    let deleted_slugs = classification.deleted_leaf_slugs;

    let deleted_set: HashSet<String> = deleted_slugs.iter().cloned().collect();

    let deleted_filenames: HashSet<String> = manifest
        .leaves
        .iter()
        .filter(|l| deleted_set.contains(l.slug.as_str()))
        .map(|l| l.file.clone())
        .collect();

    let branch_referenced_slugs: HashSet<&str> = manifest
        .branches
        .iter()
        .flat_map(|b| b.leaves.iter().map(|s| s.as_str()))
        .collect();
    let orphan_slugs: Vec<String> = deleted_slugs
        .iter()
        .filter(|s| !branch_referenced_slugs.contains(s.as_str()))
        .cloned()
        .collect();

    Ok(DeletionClassification {
        deleted_slugs,
        deleted_set,
        deleted_filenames,
        orphan_slugs,
    })
}

/// Build the orphan-prune notification, if any.
fn orphan_prune_message(orphan_slugs: &[String]) -> Option<String> {
    if orphan_slugs.is_empty() {
        return None;
    }
    let n = orphan_slugs.len();
    Some(format!(
        "pruned {} orphan leaf record{} (file{} missing, not in any branch)",
        n,
        if n == 1 { "" } else { "s" },
        if n == 1 { "" } else { "s" }
    ))
}

/// Repair branch files: prune deleted leaves, rewrite frontmatter, and
/// collect branch removals. Writes to disk happen here (frontmatter rewrites
/// only — deletes are deferred to the assembly stage).
fn repair_branch_files(
    cfg: &SeededConfig,
    manifest: &Manifest,
    class: &DeletionClassification,
) -> BranchRepairOutcome {
    let mut branches_removed = Vec::new();
    let mut branch_deletes = Vec::new();
    let mut repaired_branches = Vec::new();
    let mut repaired_branch_slugs = Vec::new();
    let mut frontmatter_notes = Vec::new();

    for branch in &manifest.branches {
        let remaining: Vec<Slug> = branch
            .leaves
            .iter()
            .filter(|s| !class.deleted_set.contains(s.as_str()))
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
                                        !class.deleted_filenames.contains(filename.as_str())
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
                            frontmatter_notes.push(note);
                        }
                    }
                }
            }
            let mut repaired = branch.clone();
            repaired.leaves = remaining;
            repaired_branches.push(repaired);
        }
    }

    BranchRepairOutcome {
        repaired_branches,
        branch_deletes,
        repaired_branch_slugs,
        branches_removed,
        frontmatter_notes,
    }
}

/// Build the "repaired N branches" summary notification, if any.
fn repaired_summary_message(slugs: &[String]) -> Option<String> {
    if slugs.is_empty() {
        return None;
    }
    let names = slugs.join(", ");
    Some(format!(
        "repaired {} branch{} with deleted leaves: {}",
        slugs.len(),
        if slugs.len() == 1 { "" } else { "es" },
        names,
    ))
}

/// Build the "removed N stale branches" summary notification, if any.
fn removed_summary_message(removed: &[RemovedBranchResult]) -> Option<String> {
    if removed.is_empty() {
        return None;
    }
    let names: Vec<&str> = removed.iter().map(|b| b.slug.as_str()).collect();
    Some(format!(
        "removed {} stale branch{} below threshold: {}",
        removed.len(),
        if removed.len() == 1 { "" } else { "es" },
        names.join(", "),
    ))
}

/// Assemble the repaired manifest: filter out deleted leaf records and
/// replace branches with the repaired set.
fn assemble_repaired_manifest(
    manifest: &Manifest,
    class: &DeletionClassification,
    outcome: &BranchRepairOutcome,
) -> Manifest {
    let repaired_leaves: Vec<Leaf> = manifest
        .leaves
        .iter()
        .filter(|l| !class.deleted_set.contains(l.slug.as_str()))
        .cloned()
        .collect();

    Manifest {
        tree: manifest.tree.clone(),
        leaves: repaired_leaves,
        branches: outcome.repaired_branches.clone(),
    }
}
