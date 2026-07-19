use super::*;
use crate::cli::synthesize::types::{SynthesisMode, SynthesisOptions};
use crate::cli::synthesize::validation::{SynthesisPlan, ValidatedBranch};
use crate::domain::slug::Slug;
use crate::domain::state::TreeState;
use crate::domain::{Branch, Timestamp, Title};

// ── helpers ───────────────────────────────────────────────────────────────────

fn fresh_state(name: &str, created_at: &str, last_synthesized_at: Option<&str>) -> TreeState {
    TreeState {
        tree: crate::domain::state::TreeMetadata {
            name: name.to_string(),
            created_at: Timestamp::parse(created_at).unwrap(),
            last_synthesized_at: last_synthesized_at.map(|s| Timestamp::parse(s).unwrap()),
        },
        leaves: Vec::new(),
        branches: Vec::new(),
    }
}

// ── run-mode selection ─────────────────────────────────────────────────────

#[test]
fn select_run_mode_forces_full_when_no_branches_exist() {
    let state = fresh_state("t", "2026-01-01T00:00:00Z", None);
    assert_eq!(
        select_run_mode(
            SynthesisOptions {
                all: false,
                ..Default::default()
            },
            &state
        ),
        SynthesisMode::Full,
        "fresh tree with no branches must use full synthesis even without --all"
    );
}

#[test]
fn select_run_mode_incremental_only_with_branches_and_no_all() {
    let mut state = fresh_state("t", "2026-01-01T00:00:00Z", Some("2026-01-02T00:00:00Z"));
    state.branches.push(Branch {
        slug: Slug::generate("existing", ""),
        file: "branch/existing.md".to_string(),
        title: Title::parse("existing").unwrap(),
        created_at: Timestamp::parse("2026-01-02T00:00:00Z").unwrap(),
        updated_at: Timestamp::parse("2026-01-02T00:00:00Z").unwrap(),
        leaves: vec![Slug::generate("a", "")],
    });

    assert_eq!(
        select_run_mode(
            SynthesisOptions {
                all: false,
                ..Default::default()
            },
            &state
        ),
        SynthesisMode::Incremental,
        "tree with branches and no --all runs incremental"
    );
    assert_eq!(
        select_run_mode(
            SynthesisOptions {
                all: true,
                ..Default::default()
            },
            &state
        ),
        SynthesisMode::Full,
        "--all always forces full"
    );
}

// ── leaf multi-membership ────────────────────────────────────────────────────

#[test]
fn build_state_delta_allows_one_leaf_in_multiple_branches() {
    let plan = SynthesisPlan {
        branches: vec![
            ValidatedBranch {
                slug: "concept-a".to_string(),
                title: "Concept A".to_string(),
                body: "body a".to_string(),
                leaves: vec!["shared-leaf.md".to_string(), "alpha.md".to_string()],
            },
            ValidatedBranch {
                slug: "concept-b".to_string(),
                title: "Concept B".to_string(),
                body: "body b".to_string(),
                leaves: vec!["shared-leaf.md".to_string(), "beta.md".to_string()],
            },
        ],
    };

    let current = fresh_state("t", "2026-01-01T00:00:00Z", None);
    let ts = Timestamp::parse("2026-06-28T00:00:00Z").unwrap();
    let delta = build_state_delta(&current, &plan, SynthesisMode::Full, &ts).unwrap();

    assert_eq!(delta.new_state.branches.len(), 2);
    assert_eq!(
        delta.branches_created.len(),
        2,
        "both branches are new in Full mode"
    );

    let shared_slug = Slug::parse("shared-leaf").unwrap();
    let containing: Vec<&str> = delta
        .new_state
        .branches_for_leaf(&shared_slug)
        .iter()
        .map(|b| b.slug.as_str())
        .collect();
    assert_eq!(
        containing,
        vec!["concept-a", "concept-b"],
        "a leaf must be allowed in multiple branches"
    );
}
