// ── compile planning: leaf loading, classification, manifest delta ─────────────

use std::collections::{HashMap, HashSet};
use std::fs;

use crate::domain::frontmatter;
use crate::domain::manifest::{Manifest, TreeMeta};
use crate::domain::slug::Slug;
use crate::domain::Timestamp;
use crate::domain::{Branch, Leaf, Title};
use crate::engine::config::SeededConfig;

use super::validation::{CompilePlan, ValidatedBranch};
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

#[derive(Debug)]
pub(super) struct PlannedBranchWrite {
    pub(super) record: Branch,
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

pub(super) fn read_valid_leaves(
    cfg: &SeededConfig,
    entries: &[Leaf],
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
                        .unwrap_or_else(|| {
                            entry
                                .title
                                .as_ref()
                                .map(|t| t.as_str().to_string())
                                .unwrap_or_default()
                        });
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
    match run_mode {
        CompileRunMode::Full => build_full_delta(current, plan, run_timestamp),
        CompileRunMode::Incremental => build_incremental_delta(current, plan, run_timestamp),
    }
}

fn build_full_delta(
    current: &Manifest,
    plan: &CompilePlan,
    run_timestamp: &Timestamp,
) -> Result<ManifestDelta, CompileError> {
    let mut branch_writes = Vec::new();
    let mut branch_deletes = Vec::new();
    let mut branches_created = Vec::new();
    let mut branches_updated = Vec::new();
    let mut new_branches = Vec::new();

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
        let existing = current.branch_by_slug_str(&planned.slug);
        let (record, result, write) = build_branch_artifacts(planned, existing, run_timestamp);
        if existing.is_some() {
            branches_updated.push(result);
        } else {
            branches_created.push(result);
        }
        branch_writes.push(write);
        new_branches.push(record);
    }

    finalize_manifest_delta(
        current,
        run_timestamp,
        branch_writes,
        branch_deletes,
        branches_created,
        branches_updated,
        new_branches,
    )
}

/// ponytail: shared artifact builder; eliminates duplicate Branch/
/// BranchResult/PlannedBranchWrite construction between full and incremental deltas.
fn build_branch_artifacts(
    planned: &ValidatedBranch,
    existing: Option<&Branch>,
    run_timestamp: &Timestamp,
) -> (Branch, BranchResult, PlannedBranchWrite) {
    let slug = Slug::parse(&planned.slug).unwrap_or_else(|_| Slug::generate(&planned.slug, ""));
    let (file, created_at) = match existing {
        Some(ex) => (ex.file.clone(), ex.created_at.clone()),
        None => (format!("branch/{}.md", planned.slug), run_timestamp.clone()),
    };
    let record = Branch {
        slug: slug.clone(),
        file,
        // ponytail: title validated non-empty in parse phase; panic on impossible empty.
        title: Title::parse(&planned.title).expect("branch title validated non-empty upstream"),
        created_at,
        updated_at: run_timestamp.clone(),
        leaves: validated_branch_leaf_slugs(planned),
    };
    let result = branch_result(
        record.slug.as_str(),
        record.title.as_str(),
        record.leaves.len(),
    );
    let write = PlannedBranchWrite {
        record: record.clone(),
        file_leaves: planned.leaves.clone(),
        body: planned.body.clone(),
    };
    (record, result, write)
}

fn build_incremental_delta(
    current: &Manifest,
    plan: &CompilePlan,
    run_timestamp: &Timestamp,
) -> Result<ManifestDelta, CompileError> {
    let mut branch_writes = Vec::new();
    let mut branches_created = Vec::new();
    let mut branches_updated = Vec::new();
    let mut new_branches = Vec::new();

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
            let (record, result, write) =
                build_branch_artifacts(planned, Some(current_branch), run_timestamp);
            branches_updated.push(result);
            branch_writes.push(write);
            new_branches.push(record);
        } else {
            new_branches.push(current_branch.clone());
        }
    }
    for planned in &plan.branches {
        if current_branch_slugs.contains(planned.slug.as_str()) {
            continue;
        }
        let (record, result, write) = build_branch_artifacts(planned, None, run_timestamp);
        branches_created.push(result);
        branch_writes.push(write);
        new_branches.push(record);
    }

    finalize_manifest_delta(
        current,
        run_timestamp,
        branch_writes,
        Vec::new(), // ponytail: incremental never deletes branches
        branches_created,
        branches_updated,
        new_branches,
    )
}

/// ponytail: shared finalizer; the Manifest construction is identical across modes.
fn finalize_manifest_delta(
    current: &Manifest,
    run_timestamp: &Timestamp,
    branch_writes: Vec<PlannedBranchWrite>,
    branch_deletes: Vec<String>,
    branches_created: Vec<BranchResult>,
    branches_updated: Vec<BranchResult>,
    new_branches: Vec<Branch>,
) -> Result<ManifestDelta, CompileError> {
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
