use super::*;
use crate::domain::{Slug, Timestamp};

fn resolution_fixture() -> Manifest {
    Manifest {
        tree: TreeMeta {
            name: "fixture".to_string(),
            created_at: Timestamp::parse("2026-05-19T13:00:00Z").unwrap(),
            last_compiled_at: Some(Timestamp::parse("2026-05-19T15:00:00Z").unwrap()),
        },
        leaves: vec![
            LeafRecord {
                slug: Slug::parse("alpha").unwrap(),
                file: "alpha.md".to_string(),
                title: ("Alpha").to_string(),
                url: ("https://example.com/a").to_string(),
                collected_at: Timestamp::parse("2026-05-19T14:00:00Z").unwrap(),
                summary: None,
            },
            LeafRecord {
                slug: Slug::parse("beta").unwrap(),
                file: "beta.md".to_string(),
                title: ("Beta").to_string(),
                url: ("https://example.com/b").to_string(),
                collected_at: Timestamp::parse("2026-05-19T14:30:00Z").unwrap(),
                summary: None,
            },
            LeafRecord {
                slug: Slug::parse("gamma").unwrap(),
                file: "gamma.md".to_string(),
                title: ("Gamma").to_string(),
                url: ("https://example.com/g").to_string(),
                collected_at: Timestamp::parse("2026-05-19T16:00:00Z").unwrap(),
                summary: None,
            },
        ],
        branches: vec![
            BranchRecord {
                slug: Slug::parse("topic-x").unwrap(),
                file: "branches/topic-x.md".to_string(),
                title: ("Topic X").to_string(),
                created_at: Timestamp::parse("2026-05-19T15:00:00Z").unwrap(),
                updated_at: Timestamp::parse("2026-05-19T15:00:00Z").unwrap(),
                leaves: vec![Slug::parse("alpha").unwrap(), Slug::parse("beta").unwrap()],
            },
            BranchRecord {
                slug: Slug::parse("topic-y").unwrap(),
                file: "branches/topic-y.md".to_string(),
                title: ("Topic Y").to_string(),
                created_at: Timestamp::parse("2026-05-19T15:00:00Z").unwrap(),
                updated_at: Timestamp::parse("2026-05-19T15:00:00Z").unwrap(),
                leaves: vec![Slug::parse("beta").unwrap()],
            },
        ],
    }
}

#[test]
fn branch_by_slug_returns_record_for_known_slug() {
    let m = resolution_fixture();
    let b = m.branch_by_slug_str("topic-x").unwrap();
    assert_eq!(b.title.as_str(), "Topic X");
}

#[test]
fn branch_by_slug_returns_none_for_unknown_slug() {
    let m = resolution_fixture();
    assert!(m.branch_by_slug_str("missing").is_none());
}

#[test]
fn uncompiled_leaves_returns_only_those_collected_after_last_compile() {
    let m = resolution_fixture();
    let uncompiled = m.uncompiled_leaves();
    assert_eq!(uncompiled.len(), 1);
    assert_eq!(uncompiled[0].slug.as_str(), "gamma");
}

#[test]
fn uncompiled_leaves_returns_all_when_never_compiled() {
    let mut m = resolution_fixture();
    m.tree.last_compiled_at = None;
    let uncompiled = m.uncompiled_leaves();
    assert_eq!(uncompiled.len(), 3);
}

#[test]
fn uncompiled_leaves_empty_when_all_predate_last_compile() {
    let mut m = resolution_fixture();
    m.leaves.retain(|l| l.slug.as_str() != "gamma");
    let uncompiled = m.uncompiled_leaves();
    assert!(uncompiled.is_empty());
}

#[test]
fn branches_for_leaf_returns_multiple_when_shared() {
    let m = resolution_fixture();
    let branches = m.branches_for_leaf(&Slug::parse("beta").unwrap());
    let slugs: Vec<&str> = branches.iter().map(|b| b.slug.as_str()).collect();
    assert_eq!(slugs, vec!["topic-x", "topic-y"]);
}

#[test]
fn branches_for_leaf_returns_singleton_when_only_one_branch_owns_it() {
    let m = resolution_fixture();
    let branches = m.branches_for_leaf(&Slug::parse("alpha").unwrap());
    let slugs: Vec<&str> = branches.iter().map(|b| b.slug.as_str()).collect();
    assert_eq!(slugs, vec!["topic-x"]);
}

#[test]
fn branches_for_leaf_returns_empty_for_unknown_leaf() {
    let m = resolution_fixture();
    assert!(m
        .branches_for_leaf(&Slug::parse("nope").unwrap())
        .is_empty());
}
