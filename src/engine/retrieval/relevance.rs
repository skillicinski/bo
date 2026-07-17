// retrieval/relevance — diagnostics computation + low-relevance gating.

use super::terms::{count_term_hits_in_tokens, is_generic_term, tokenize, unique_terms};
use super::{LowRelevanceReason, RetrievalDiagnostics, RetrievalError, RetrievedDoc};

const MIN_SINGLE_TERM_DENSITY: f64 = 20.0;
const MIN_MULTI_TERM_DENSITY: f64 = 8.0;
const MOSTLY_GENERIC_RATIO_NUMERATOR: usize = 2;
const MOSTLY_GENERIC_RATIO_DENOMINATOR: usize = 3;

pub(super) fn compute_retrieval_diagnostics(
    title: &str,
    summary: &str,
    body: &str,
    terms: &[String],
) -> RetrievalDiagnostics {
    let title_tokens = tokenize(title);
    let summary_tokens = tokenize(summary);
    let body_tokens = tokenize(body);
    let unique_terms = unique_terms(terms);

    let mut diagnostics = RetrievalDiagnostics {
        token_count: title_tokens.len() + summary_tokens.len() + body_tokens.len(),
        ..RetrievalDiagnostics::default()
    };

    for term in unique_terms {
        let title_hits = count_term_hits_in_tokens(&title_tokens, term);
        let summary_hits = count_term_hits_in_tokens(&summary_tokens, term);
        let body_hits = count_term_hits_in_tokens(&body_tokens, term);
        let term_hits = title_hits + summary_hits + body_hits;

        if term_hits > 0 {
            diagnostics.matched_terms += 1;
            if !is_generic_term(term) {
                diagnostics.matched_non_generic_terms += 1;
            }
        }

        if !is_generic_term(term) {
            diagnostics.title_summary_non_generic_hits += title_hits + summary_hits;
        }

        diagnostics.title_hits += title_hits;
        diagnostics.summary_hits += summary_hits;
        diagnostics.body_hits += body_hits;
        diagnostics.total_hits += term_hits;
    }

    diagnostics
}

pub fn validate_relevance(terms: &[String], docs: &[RetrievedDoc]) -> Result<(), RetrievalError> {
    if docs.is_empty() {
        return Err(RetrievalError::NoResults);
    }

    let matched_sources = docs.len();

    if is_mostly_generic(terms) && !docs.iter().any(|doc| is_focused_generic_match(doc, terms)) {
        return Err(RetrievalError::LowRelevance {
            reason: LowRelevanceReason::GenericTerms,
            matched_sources,
        });
    }

    if !docs.iter().any(|doc| is_strong_relevance_match(doc, terms)) {
        return Err(RetrievalError::LowRelevance {
            reason: LowRelevanceReason::WeakMatches,
            matched_sources,
        });
    }

    Ok(())
}

fn is_mostly_generic(terms: &[String]) -> bool {
    let unique_terms = unique_terms(terms);
    if unique_terms.is_empty() {
        return false;
    }

    let generic_terms = unique_terms
        .iter()
        .filter(|term| is_generic_term(term))
        .count();

    generic_terms * MOSTLY_GENERIC_RATIO_DENOMINATOR
        >= unique_terms.len() * MOSTLY_GENERIC_RATIO_NUMERATOR
}

fn is_focused_generic_match(doc: &RetrievedDoc, terms: &[String]) -> bool {
    let unique_term_count = unique_terms(terms).len();
    if unique_term_count == 0 {
        return false;
    }

    let required_terms = unique_term_count.min(2);
    let title_summary_hits = doc.diagnostics.title_hits + doc.diagnostics.summary_hits;

    doc.diagnostics.matched_terms >= required_terms && title_summary_hits >= required_terms
}

fn is_strong_relevance_match(doc: &RetrievedDoc, terms: &[String]) -> bool {
    let diagnostics = &doc.diagnostics;
    if diagnostics.matched_terms == 0 || diagnostics.total_hits == 0 {
        return false;
    }

    let unique_terms = unique_terms(terms);
    let unique_term_count = unique_terms.len();
    let non_generic_term_count = unique_terms
        .iter()
        .filter(|term| !is_generic_term(term))
        .count();
    let title_summary_hits = diagnostics.title_hits + diagnostics.summary_hits;
    let density = if diagnostics.token_count == 0 {
        0.0
    } else {
        (diagnostics.total_hits as f64 * 1000.0) / diagnostics.token_count as f64
    };

    if unique_term_count == 1 {
        let term = unique_terms[0];
        return !is_generic_term(term)
            && (title_summary_hits > 0
                || diagnostics.total_hits >= 2
                || density >= MIN_SINGLE_TERM_DENSITY);
    }

    if non_generic_term_count == 1
        && diagnostics.matched_non_generic_terms == 1
        && diagnostics.title_summary_non_generic_hits > 0
    {
        return true;
    }

    if non_generic_term_count > 1
        && diagnostics.matched_non_generic_terms >= non_generic_term_count.min(2)
        && (diagnostics.title_summary_non_generic_hits > 0 || density >= MIN_MULTI_TERM_DENSITY)
    {
        return true;
    }

    diagnostics.matched_terms >= unique_term_count.min(2)
        && (title_summary_hits > 0 || density >= MIN_MULTI_TERM_DENSITY)
}

#[cfg(test)]
#[path = "../../tests/engine_retrieval/relevance_tests.rs"]
mod tests;
