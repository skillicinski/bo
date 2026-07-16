// Collect stage: classify and expand inputs into compute-ready items.
//
// Single URLs pass through; `.txt` URL-list files fan out one entry per
// non-empty line; existing local `.md` files route to the note path. Each
// expanded item is what the later dedup/compute/commit phases consume.

use std::fs;
use std::path::Path;

#[derive(Debug, Clone)]
pub(super) enum ExpandedCollectInput {
    Url {
        input: String,
        url: String,
    },
    Note {
        input: String,
        path: String,
    },
    Failure {
        input: String,
        code: String,
        message: String,
    },
}

pub(super) fn expand_collect_inputs(inputs: &[String]) -> Vec<ExpandedCollectInput> {
    inputs
        .iter()
        .flat_map(|input| expand_collect_input(input))
        .collect()
}

fn expand_collect_input(input: &str) -> Vec<ExpandedCollectInput> {
    if is_local_note_file(input) {
        return vec![ExpandedCollectInput::Note {
            input: input.to_string(),
            path: input.to_string(),
        }];
    }
    if !is_url_list_file(input) {
        return vec![ExpandedCollectInput::Url {
            input: input.to_string(),
            url: input.to_string(),
        }];
    }

    let contents = match fs::read_to_string(input) {
        Ok(contents) => contents,
        Err(error) => {
            return vec![ExpandedCollectInput::Failure {
                input: input.to_string(),
                code: "url_list_read_error".to_string(),
                message: format!("failed to read URL list: {error}"),
            }]
        }
    };

    let mut urls = Vec::new();
    for (line_index, line) in contents.lines().enumerate() {
        let url = line.trim();
        if url.is_empty() {
            continue;
        }
        urls.push(ExpandedCollectInput::Url {
            input: format!("{}:{}", input, line_index + 1),
            url: url.to_string(),
        });
    }

    if urls.is_empty() {
        urls.push(ExpandedCollectInput::Failure {
            input: input.to_string(),
            code: "empty_url_list".to_string(),
            message: "URL list file contains no URLs".to_string(),
        });
    }

    urls
}

fn is_url_list_file(input: &str) -> bool {
    let path = Path::new(input);
    let has_txt_extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("txt"));
    if !has_txt_extension {
        return false;
    }
    // A URL containing :// is never a local URL list file.
    if input.contains("://") {
        return false;
    }
    // Bare domains ending in .txt (e.g. example.com/urls.txt) should not be
    // mistaken for local .txt files. If the part before the first '/' looks
    // like a hostname (contains a dot), treat the input as a URL.
    let before_slash = input.split('/').next().unwrap_or(input);
    if before_slash.contains('.') {
        return false;
    }
    true
}

/// A local markdown note: `.md` extension (case-insensitive), no URL scheme,
/// and the file exists on disk. Existence naturally excludes bare domains
/// and `https://.../x.md` URLs.
pub(super) fn is_local_note_file(input: &str) -> bool {
    if input.contains("://") {
        return false;
    }
    let path = Path::new(input);
    let is_md = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
    is_md && path.is_file()
}

/// Whether `inputs` selects the single-result output contract. A lone
/// argument that is neither a URL list nor an existing local note is
/// collected as a single URL, selecting `CollectOutput::Single`; every other
/// input shape returns a batch.
///
/// The URL-list test must agree with `is_url_list_file` (what expansion uses):
/// anything expansion reads as a list must select Batch, or `shape_single`
/// would report only the first outcome — or panic on an empty/failed list.
/// `ends_with(".txt")` (case-sensitive) keeps the pre-unification routing for
/// bare lowercase `urls.txt`-style arguments that `is_url_list_file` rejects
/// via its dot/host heuristic; `is_url_list_file` (case-insensitive) covers
/// nested and mixed-case (`.TXT`) lists it accepts.
pub fn is_single_bare_url(inputs: &[String]) -> bool {
    if inputs.len() != 1 {
        return false;
    }
    let input = &inputs[0];
    let is_url_list_like =
        (input.ends_with(".txt") && !input.contains("://")) || is_url_list_file(input);
    !is_url_list_like && !is_local_note_file(input)
}

#[cfg(test)]
#[path = "../../tests/cli_collect_input_tests.rs"]
mod tests;
