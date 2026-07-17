// retrieval/terms — term extraction, tokenization, generic-term classification.

use super::RetrievalError;
use std::collections::HashSet;

const STOP_WORDS: &[&str] = &[
    "what", "which", "who", "whom", "where", "when", "why", "how", "is", "are", "was", "were",
    "am", "do", "does", "did", "has", "have", "had", "can", "could", "would", "should", "will",
    "shall", "the", "a", "an", "of", "in", "on", "at", "to", "for", "with", "by", "from", "about",
    "between", "and", "or", "but", "not", "no", "if", "then", "than", "that", "this", "these",
    "those", "it", "its", "be", "been", "being", "my", "your", "our", "their", "me", "you", "us",
    "them", "he", "she", "we", "they", "his", "her",
];

/// Extract meaningful search terms from a natural-language question.
/// Strips stop words, possessives, boundary punctuation, and terms < 2 chars.
pub fn extract_terms(question: &str) -> Result<Vec<String>, RetrievalError> {
    let terms: Vec<String> = question
        .split_whitespace()
        .map(strip_punctuation)
        .map(|w| strip_possessive(&w))
        .map(|w| w.to_lowercase())
        .filter(|w| w.len() >= 2)
        .filter(|w| !STOP_WORDS.contains(&w.as_str()))
        .collect();

    if terms.is_empty() {
        return Err(RetrievalError::NoTerms);
    }
    Ok(terms)
}

/// Strip leading/trailing punctuation from a word.
fn strip_punctuation(word: &str) -> String {
    word.trim_matches(|c: char| c.is_ascii_punctuation())
        .to_string()
}

/// Strip common possessive/contraction suffixes: 's, 't, 're, 've, 'd, 'll
fn strip_possessive(word: &str) -> String {
    for suffix in &[
        "'s",
        "'t",
        "'re",
        "'ve",
        "'d",
        "'ll",
        "\u{2019}s",
        "\u{2019}t",
    ] {
        if let Some(stem) = word.strip_suffix(suffix) {
            if !stem.is_empty() {
                return stem.to_string();
            }
        }
    }
    word.to_string()
}

pub(crate) fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in input.chars() {
        for lower in ch.to_lowercase() {
            if lower.is_alphanumeric() {
                current.push(lower);
            } else if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

pub(super) fn unique_terms(terms: &[String]) -> Vec<&str> {
    let mut seen = HashSet::new();
    let mut unique = Vec::new();

    for term in terms {
        if seen.insert(term.as_str()) {
            unique.push(term.as_str());
        }
    }

    unique
}

pub(super) fn count_term_hits_in_tokens(tokens: &[String], term: &str) -> usize {
    let term_tokens = tokenize(term);
    match term_tokens.len() {
        0 => 0,
        1 => tokens
            .iter()
            .filter(|token| token.as_str() == term_tokens[0].as_str())
            .count(),
        n if n <= tokens.len() => tokens
            .windows(n)
            .filter(|window| {
                window
                    .iter()
                    .map(String::as_str)
                    .eq(term_tokens.iter().map(String::as_str))
            })
            .count(),
        _ => 0,
    }
}

pub(super) fn is_generic_term(term: &str) -> bool {
    matches!(
        term,
        "important"
            | "system"
            | "systems"
            | "pattern"
            | "patterns"
            | "concept"
            | "concepts"
            | "model"
            | "models"
            | "approach"
            | "approaches"
            | "method"
            | "methods"
            | "topic"
            | "topics"
            | "source"
            | "sources"
            | "information"
            | "details"
            | "example"
            | "examples"
            | "data"
            | "content"
            | "use"
            | "uses"
            | "using"
            | "used"
            | "work"
            | "works"
            | "benefit"
            | "benefits"
            | "tradeoff"
            | "tradeoffs"
            | "good"
            | "bad"
            | "best"
            | "common"
            | "general"
            | "overview"
            | "summary"
            | "guide"
    )
}

#[cfg(test)]
#[path = "../../tests/engine_retrieval/terms_tests.rs"]
mod tests;
