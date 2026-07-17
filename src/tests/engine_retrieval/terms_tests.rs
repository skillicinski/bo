use crate::engine::retrieval::{extract_terms, RetrievalError};

#[test]
fn extract_basic_question() {
    let terms = extract_terms("what are the tradeoffs of Rust's ownership model?").unwrap();
    assert_eq!(terms, vec!["tradeoffs", "rust", "ownership", "model"]);
}

#[test]
fn extract_single_word() {
    let terms = extract_terms("ownership").unwrap();
    assert_eq!(terms, vec!["ownership"]);
}

#[test]
fn extract_all_stop_words_returns_error() {
    let err = extract_terms("what is it?").unwrap_err();
    assert!(matches!(err, RetrievalError::NoTerms));
}

#[test]
fn extract_strips_possessives() {
    let terms = extract_terms("Rust's borrow checker").unwrap();
    assert_eq!(terms, vec!["rust", "borrow", "checker"]);
}

#[test]
fn extract_drops_short_terms() {
    // "a" and "I" are < 2 chars and should be dropped
    let terms = extract_terms("a big I see").unwrap();
    assert_eq!(terms, vec!["big", "see"]);
}

#[test]
fn extract_strips_boundary_punctuation() {
    let terms = extract_terms("(memory) safety! \"lifetimes\"").unwrap();
    assert_eq!(terms, vec!["memory", "safety", "lifetimes"]);
}

#[test]
fn extract_unicode_possessive() {
    // Smart quote possessive: Rust\u{2019}s
    let terms = extract_terms("Rust\u{2019}s ownership").unwrap();
    assert_eq!(terms, vec!["rust", "ownership"]);
}
