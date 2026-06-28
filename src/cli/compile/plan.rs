// ── compile planning: leaf loading, classification, manifest delta ─────────────

use std::collections::{HashMap, HashSet};
use std::fs;

use crate::domain::frontmatter;
use crate::domain::manifest::{BranchRecord, LeafRecord, Manifest, TreeMeta};
use crate::domain::slug::Slug;
use crate::domain::Timestamp;
use crate::engine::config::SeededConfig;

use super::parse::{CompilePlan, ValidatedBranch};
use super::{BranchResult, CompileError, CompileOptions, CompileRunMode};

// ── types ─────────────────────────────────────────────────────────────────────

/// A leaf with its full content loaded for prompt assembly.
pub(super) struct LoadedLeaf {
    pub(super) slug: String,
    pub(super) filename: String,
    pub(super) title: String,
    pub(super) summary: Option<String>,
    pub(super) body: String,
    pub(super) collected_at: String,
}

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

#[derive(Debug)]
pub(super) struct PlannedBranchWrite {
    pub(super) record: BranchRecord,
    pub(super) file_leaves: Vec<String>,
    pub(super) body: String,
}

#[derive(Debug)]
pub(super) struct ManifestDelta {
    pub(super) new_manifest: Manifest,
    pub(super) branch_writes: Vec<PlannedBranchWrite>,
    pub(super) branch_deletes: Vec<String>,
    pub(super) branches_created: Vec<BranchResult>,
    pub(super) branches_updated: Vec<BranchResult>,
}

// ── functions ─────────────────────────────────────────────────────────────────

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

    let repaired_leaves: Vec<LeafRecord> = manifest
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
    crate::domain::manifest::write(&manifest_path, &repaired_manifest)
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
/// Decide whether this compile re-derives the whole branch graph (`Full`) or
/// fits new leaves into existing branches (`Incremental`).
///
/// A tree with no branches has nothing to incrementally update, so it compiles
/// from scratch regardless of `--all`. Incremental mode is only coherent
/// against an existing branch graph.
pub(super) fn select_run_mode(options: CompileOptions, manifest: &Manifest) -> CompileRunMode {
    if options.all || manifest.branches.is_empty() {
        CompileRunMode::Full
    } else {
        CompileRunMode::Incremental
    }
}

pub(super) fn select_new_leaf_slugs(manifest: &Manifest) -> Result<Vec<String>, CompileError> {
    let Some(last_compiled_at) = &manifest.tree.last_compiled_at else {
        return Ok(manifest
            .leaves
            .iter()
            .map(|leaf| leaf.slug.as_str().to_string())
            .collect());
    };

    Ok(manifest
        .leaves
        .iter()
        .filter(|leaf| &leaf.collected_at > last_compiled_at)
        .map(|leaf| leaf.slug.as_str().to_string())
        .collect())
}

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

pub(super) fn read_valid_leaves(
    cfg: &SeededConfig,
    entries: &[LeafRecord],
) -> (Vec<LoadedLeaf>, Vec<String>) {
    let tree = cfg.tree();
    let mut loaded = Vec::new();
    let mut skipped = Vec::new();

    for entry in entries {
        let leaf_path = tree.join(&entry.file);
        match fs::read_to_string(&leaf_path) {
            Ok(content) => match frontmatter::parse(&content) {
                Ok((mapping, body)) => {
                    let title = mapping
                        .get("title")
                        .and_then(|v| v.as_str())
                        .filter(|title| !title.trim().is_empty())
                        .map(str::to_string)
                        .unwrap_or_else(|| entry.title.as_str().to_string());
                    loaded.push(LoadedLeaf {
                        slug: entry.slug.as_str().to_string(),
                        filename: entry.file.clone(),
                        title,
                        summary: entry.summary.clone(),
                        body,
                        collected_at: entry.collected_at.to_rfc3339_millis(),
                    });
                }
                Err(_) => skipped.push(entry.file.clone()),
            },
            Err(_) => skipped.push(entry.file.clone()),
        }
    }

    (loaded, skipped)
}

pub(super) fn branch_result(slug: &str, title: &str, leaf_count: usize) -> BranchResult {
    BranchResult {
        slug: slug.to_string(),
        title: title.to_string(),
        leaf_count,
    }
}

pub(super) fn validated_branch_leaf_slugs(branch: &ValidatedBranch) -> Vec<Slug> {
    branch
        .leaves
        .iter()
        .map(|leaf| {
            let stem = leaf.strip_suffix(".md").unwrap_or(leaf);
            Slug::parse(stem).unwrap_or_else(|_| Slug::generate(stem, ""))
        })
        .collect()
}

pub(super) fn build_manifest_delta(
    current: &Manifest,
    plan: &CompilePlan,
    run_mode: CompileRunMode,
    run_timestamp: &Timestamp,
) -> Result<ManifestDelta, CompileError> {
    let mut branch_writes = Vec::new();
    let mut branch_deletes = Vec::new();
    let mut branches_created = Vec::new();
    let mut branches_updated = Vec::new();
    let mut new_branches = Vec::new();

    match run_mode {
        CompileRunMode::Full => {
            let planned_slugs: HashSet<&str> = plan
                .branches
                .iter()
                .map(|branch| branch.slug.as_str())
                .collect();
            for branch in &current.branches {
                if !planned_slugs.contains(branch.slug.as_str()) {
                    branch_deletes.push(branch.file.clone());
                }
            }
            for planned in &plan.branches {
                let created_at = current
                    .branch_by_slug_str(&planned.slug)
                    .map(|branch| branch.created_at.clone())
                    .unwrap_or_else(|| run_timestamp.clone());
                let slug = Slug::parse(&planned.slug)
                    .unwrap_or_else(|_| Slug::generate(&planned.slug, ""));
                let record = BranchRecord {
                    slug: slug.clone(),
                    file: format!("branches/{}.md", planned.slug),
                    title: planned.title.clone(),
                    created_at,
                    updated_at: run_timestamp.clone(),
                    leaves: validated_branch_leaf_slugs(planned),
                };
                if current.branch_by_slug_str(&planned.slug).is_some() {
                    branches_updated.push(branch_result(
                        record.slug.as_str(),
                        record.title.as_str(),
                        record.leaves.len(),
                    ));
                    branch_writes.push(PlannedBranchWrite {
                        record: record.clone(),
                        file_leaves: planned.leaves.clone(),
                        body: planned.body.clone(),
                    });
                } else {
                    branches_created.push(branch_result(
                        record.slug.as_str(),
                        record.title.as_str(),
                        record.leaves.len(),
                    ));
                    branch_writes.push(PlannedBranchWrite {
                        record: record.clone(),
                        file_leaves: planned.leaves.clone(),
                        body: planned.body.clone(),
                    });
                }
                new_branches.push(record);
            }
        }
        CompileRunMode::Incremental => {
            let planned_by_slug: HashMap<&str, &ValidatedBranch> = plan
                .branches
                .iter()
                .map(|branch| (branch.slug.as_str(), branch))
                .collect();
            let current_branch_slugs: HashSet<&str> = current
                .branches
                .iter()
                .map(|branch| branch.slug.as_str())
                .collect();
            for current_branch in &current.branches {
                if let Some(planned) = planned_by_slug.get(current_branch.slug.as_str()) {
                    let record = BranchRecord {
                        slug: current_branch.slug.clone(),
                        file: current_branch.file.clone(),
                        title: planned.title.clone(),
                        created_at: current_branch.created_at.clone(),
                        updated_at: run_timestamp.clone(),
                        leaves: validated_branch_leaf_slugs(planned),
                    };
                    let result = branch_result(
                        record.slug.as_str(),
                        record.title.as_str(),
                        record.leaves.len(),
                    );
                    branches_updated.push(result.clone());
                    branch_writes.push(PlannedBranchWrite {
                        record: record.clone(),
                        file_leaves: planned.leaves.clone(),
                        body: planned.body.clone(),
                    });
                    new_branches.push(record);
                } else {
                    new_branches.push(current_branch.clone());
                }
            }
            for planned in &plan.branches {
                if current_branch_slugs.contains(planned.slug.as_str()) {
                    continue;
                }
                let slug = Slug::parse(&planned.slug)
                    .unwrap_or_else(|_| Slug::generate(&planned.slug, ""));
                let record = BranchRecord {
                    slug: slug.clone(),
                    file: format!("branches/{}.md", planned.slug),
                    title: planned.title.clone(),
                    created_at: run_timestamp.clone(),
                    updated_at: run_timestamp.clone(),
                    leaves: validated_branch_leaf_slugs(planned),
                };
                branches_created.push(branch_result(
                    record.slug.as_str(),
                    record.title.as_str(),
                    record.leaves.len(),
                ));
                branch_writes.push(PlannedBranchWrite {
                    record: record.clone(),
                    file_leaves: planned.leaves.clone(),
                    body: planned.body.clone(),
                });
                new_branches.push(record);
            }
        }
    }

    let new_manifest = Manifest {
        tree: TreeMeta {
            name: current.tree.name.clone(),
            created_at: current.tree.created_at.clone(),
            last_compiled_at: Some(run_timestamp.clone()),
        },
        leaves: current.leaves.clone(),
        branches: new_branches,
    };

    Ok(ManifestDelta {
        new_manifest,
        branch_writes,
        branch_deletes,
        branches_created,
        branches_updated,
    })
}
