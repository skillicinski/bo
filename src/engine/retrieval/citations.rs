// retrieval/citations — synthesis response + wikilink/citation validation.

use super::{Citation, RetrievedDoc, SynthesisResponse};
use std::collections::HashSet;

/// Validate citations against the retrieval set.
/// Strips invalid slugs from cited_slugs and removes invalid [[slug]] from prose.
pub fn validate_citations(
    response: SynthesisResponse,
    retrieved: &[RetrievedDoc],
) -> (String, Vec<Citation>) {
    let valid_slugs: HashSet<&str> = retrieved.iter().map(|l| l.slug.as_str()).collect();

    let (answer, prose_slugs) =
        sanitize_wikilinks_and_collect_valid(&response.answer, &valid_slugs);

    let mut ordered_slugs: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for slug in prose_slugs.into_iter().chain(response.cited_slugs) {
        if valid_slugs.contains(slug.as_str()) && seen.insert(slug.clone()) {
            ordered_slugs.push(slug);
        }
    }

    let citations: Vec<Citation> = ordered_slugs
        .iter()
        .filter_map(|slug| {
            retrieved
                .iter()
                .find(|l| l.slug == *slug)
                .map(|l| Citation {
                    slug: l.slug.clone(),
                    title: l.title.clone(),
                    file: l.file.clone(),
                })
        })
        .collect();

    (answer, citations)
}

fn sanitize_wikilinks_and_collect_valid(
    answer: &str,
    valid_slugs: &HashSet<&str>,
) -> (String, Vec<String>) {
    let mut sanitized = String::with_capacity(answer.len());
    let mut valid_in_prose = Vec::new();
    let mut i = 0;

    while i < answer.len() {
        let rest = &answer[i..];
        if !rest.starts_with("[[") {
            let ch = rest.chars().next().expect("non-empty slice");
            sanitized.push(ch);
            i += ch.len_utf8();
            continue;
        }

        let Some(relative_end) = rest[2..].find("]]") else {
            sanitized.push_str(rest);
            break;
        };
        let inner_start = i + 2;
        let inner_end = inner_start + relative_end;
        let span_end = inner_end + 2;
        let inner = &answer[inner_start..inner_end];
        let span = &answer[i..span_end];

        if inner.is_empty() || inner.contains('[') || inner.contains(']') {
            sanitized.push_str(span);
        } else if valid_slugs.contains(inner) {
            sanitized.push_str(span);
            valid_in_prose.push(inner.to_string());
        } else {
            sanitized.push_str(inner);
        }

        i = span_end;
    }

    (sanitized, valid_in_prose)
}

#[cfg(test)]
#[path = "../../tests/engine_retrieval/citations_tests.rs"]
mod tests;
