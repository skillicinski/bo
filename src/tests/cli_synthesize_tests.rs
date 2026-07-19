use super::{
    degenerate_result_warning, render_human, BranchResult, SynthesisMode, SynthesisResult,
};

// ── degenerate result warning ────────────────────────────────────────────

fn branch_result(slug: &str, leaf_count: usize) -> BranchResult {
    BranchResult {
        slug: slug.to_string(),
        title: slug.to_string(),
        leaf_count,
    }
}

#[test]
fn degenerate_warning_when_single_branch_for_many_leaves() {
    let warning = degenerate_result_warning(
        Some(SynthesisMode::Full),
        &[branch_result("catch-all", 2)],
        64,
    );
    let msg = warning.expect("expected a degenerate warning");
    assert!(msg.contains("degenerate synthesis result"));
    assert!(msg.contains("1 branch"));
    assert!(msg.contains("64 leaves"));
}

#[test]
fn degenerate_warning_when_most_leaves_unbranched() {
    let warning = degenerate_result_warning(
        Some(SynthesisMode::Full),
        &[
            branch_result("a", 2),
            branch_result("b", 2),
            branch_result("c", 1),
        ],
        30,
    );
    let msg = warning.expect("expected a degenerate warning");
    assert!(msg.contains("degenerate synthesis result"));
    assert!(msg.contains("25 of 30 leaves unbranched"));
}

#[test]
fn no_degenerate_warning_for_normal_full_synthesis() {
    let warning = degenerate_result_warning(
        Some(SynthesisMode::Full),
        &[
            branch_result("a", 10),
            branch_result("b", 10),
            branch_result("c", 8),
        ],
        30,
    );
    assert!(warning.is_none());
}

#[test]
fn no_degenerate_warning_for_small_corpus() {
    let warning = degenerate_result_warning(Some(SynthesisMode::Full), &[], 20);
    assert!(warning.is_none());
}

#[test]
fn no_degenerate_warning_for_incremental_mode() {
    let warning = degenerate_result_warning(
        Some(SynthesisMode::Incremental),
        &[branch_result("single", 2)],
        64,
    );
    assert!(warning.is_none());
}

#[test]
fn degenerate_warning_low_coverage_ratio() {
    let warning = degenerate_result_warning(
        Some(SynthesisMode::Full),
        &[branch_result("concept-a", 7), branch_result("concept-b", 8)],
        66,
    );
    let msg = warning.expect("expected a degenerate warning from low coverage ratio");
    assert!(msg.contains("degenerate synthesis result"));
    assert!(msg.contains("only 15 of 66 leaves placed in branches"));
}

#[test]
fn no_degenerate_warning_for_healthy_coverage() {
    let warning = degenerate_result_warning(
        Some(SynthesisMode::Full),
        &[
            branch_result("a", 10),
            branch_result("b", 9),
            branch_result("c", 7),
        ],
        30,
    );
    assert!(warning.is_none());
}

#[test]
fn degenerate_warning_single_branch_regression() {
    let warning = degenerate_result_warning(
        Some(SynthesisMode::Full),
        &[branch_result("catch-all", 2)],
        64,
    );
    assert!(warning.is_some());
}

#[test]
fn degenerate_warning_unbranched_regression() {
    let warning = degenerate_result_warning(
        Some(SynthesisMode::Full),
        &[
            branch_result("a", 2),
            branch_result("b", 2),
            branch_result("c", 1),
        ],
        30,
    );
    assert!(warning.is_some());
}

#[test]
fn human_output_includes_notifications() {
    let result = SynthesisResult {
        status: "noop".to_string(),
        reason: Some("empty_tree".to_string()),
        mode: None,
        model: None,
        branches: Vec::new(),
        leaves_processed: 0,
        leaves_skipped: Vec::new(),
        notifications: vec![
            "pruned 1 orphan leaf record (file missing, not in any branch)".to_string(),
        ],
        warnings: Vec::new(),
    };
    let mut stdout = Vec::new();
    render_human(&result, &mut stdout, "test-tree").unwrap();
    let output = String::from_utf8(stdout).unwrap();

    assert!(output.contains("test-tree is empty"));
    assert!(output.contains("\u{2192} pruned 1 orphan"));
}
