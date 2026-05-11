// Content extraction via trafilatura

use std::fmt;
use trafilatura::{extract, Options};

pub struct ExtractedContent {
    pub title: Option<String>,
    pub body_markdown: String,
}

#[derive(Debug)]
pub enum ExtractError {
    ExtractionFailed(String),
    EmptyContent,
}

impl fmt::Display for ExtractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtractError::ExtractionFailed(msg) => write!(f, "extraction failed: {}", msg),
            ExtractError::EmptyContent => write!(f, "no content extracted"),
        }
    }
}

/// Minimum content length to consider extraction successful.
/// Below this threshold, the page is likely boilerplate (login wall, nav-only, etc.)
const MIN_CONTENT_LENGTH: usize = 50;

/// Extract readable content from raw HTML.
/// Returns markdown body with links stripped to plain text.
pub fn extract_content(html: &str) -> Result<ExtractedContent, ExtractError> {
    let opts = Options::default();
    let result = extract(html, &opts).map_err(|e| ExtractError::ExtractionFailed(e.to_string()))?;

    let body = result.content_markdown();
    let title = choose_title(
        if result.metadata.title.is_empty() {
            None
        } else {
            Some(result.metadata.title.as_str())
        },
        &body,
    );

    // Strip leading H1 if it matches the selected title — we add our own in the markdown template
    let body = if let Some(title_str) = title.as_deref() {
        strip_leading_h1(&body, title_str)
    } else {
        body
    };

    if body.trim().len() < MIN_CONTENT_LENGTH {
        return Err(ExtractError::EmptyContent);
    }

    // Post-process: strip any remaining markdown links [text](url) → text
    let body = strip_markdown_links(&body);

    Ok(ExtractedContent {
        title,
        body_markdown: body,
    })
}

fn choose_title(metadata_title: Option<&str>, body_markdown: &str) -> Option<String> {
    let metadata_title = metadata_title
        .map(str::trim)
        .filter(|title| !title.is_empty());

    if let Some(title) = metadata_title {
        if !is_clearly_chrome_title(title) {
            return Some(title.to_string());
        }
    }

    if let Some(heading) = first_meaningful_heading(body_markdown) {
        return Some(heading);
    }

    metadata_title.map(str::to_string)
}

fn first_meaningful_heading(body_markdown: &str) -> Option<String> {
    for line in body_markdown.lines() {
        let trimmed = line.trim_start();
        let heading = if let Some(rest) = trimmed.strip_prefix("# ") {
            rest
        } else if let Some(rest) = trimmed.strip_prefix("## ") {
            rest
        } else {
            continue;
        };

        let heading = strip_markdown_links(heading).trim().to_string();
        if !heading.is_empty() && !is_clearly_chrome_title(&heading) {
            return Some(heading);
        }
    }
    None
}

fn is_clearly_chrome_title(title: &str) -> bool {
    normalize_title(title) == "keyboard shortcuts"
}

fn normalize_title(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Remove a leading `# Title` line from markdown body if it matches the page title.
/// Prevents duplicate headings since we add our own `# Title` in the template.
fn strip_leading_h1(body: &str, title: &str) -> String {
    let trimmed = body.trim_start();
    // Check for "# Title" or "# Title\n"
    if let Some(rest) = trimmed.strip_prefix("# ") {
        // Find the end of the first line
        let (first_line, remainder) = match rest.find('\n') {
            Some(pos) => (&rest[..pos], &rest[pos + 1..]),
            None => (rest, ""),
        };
        // Compare case-insensitively, trimming whitespace
        if first_line.trim().eq_ignore_ascii_case(title.trim()) {
            return remainder.to_string();
        }
    }
    body.to_string()
}

/// Strip markdown links [text](url) to just text.
/// Handles nested brackets conservatively.
fn strip_markdown_links(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if chars[i] == '[' {
            // Try to find matching ] followed by (
            if let Some((text, end)) = parse_md_link(&chars, i) {
                result.push_str(&text);
                i = end;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }

    result
}

fn parse_md_link(chars: &[char], start: usize) -> Option<(String, usize)> {
    // Find closing ]
    let mut depth = 0;
    let mut j = start;
    while j < chars.len() {
        match chars[j] {
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
        j += 1;
    }
    if depth != 0 || j + 1 >= chars.len() || chars[j + 1] != '(' {
        return None;
    }

    let text: String = chars[start + 1..j].iter().collect();

    // Find closing )
    let paren_start = j + 2;
    let mut k = paren_start;
    let mut paren_depth = 1;
    while k < chars.len() {
        match chars[k] {
            '(' => paren_depth += 1,
            ')' => {
                paren_depth -= 1;
                if paren_depth == 0 {
                    return Some((text, k + 1));
                }
            }
            _ => {}
        }
        k += 1;
    }
    None
}

#[cfg(test)]
#[path = "../tests/engine_extract_tests.rs"]
mod tests;
