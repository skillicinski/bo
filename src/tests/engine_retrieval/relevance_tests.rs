use super::*;

#[test]
fn diagnostics_use_token_matches_not_substrings() {
    let terms = vec!["rust".to_string()];
    let diagnostics = compute_retrieval_diagnostics(
        "Trust Building",
        "Trustworthy teams",
        "A trust exercise",
        &terms,
    );

    assert_eq!(diagnostics.total_hits, 0);
    assert_eq!(diagnostics.matched_terms, 0);
}

#[test]
fn diagnostics_capture_focused_title_and_summary_matches() {
    let terms = vec!["rust".to_string(), "safety".to_string()];
    let diagnostics = compute_retrieval_diagnostics(
        "Rust Safety",
        "Rust ownership safety",
        "Memory safety without a garbage collector",
        &terms,
    );

    assert_eq!(diagnostics.matched_terms, 2);
    assert_eq!(diagnostics.title_hits, 2);
    assert_eq!(diagnostics.summary_hits, 2);
    assert_eq!(diagnostics.matched_non_generic_terms, 2);
}
