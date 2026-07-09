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
    let mut result =
        extract(html, &opts).map_err(|e| ExtractError::ExtractionFailed(e.to_string()))?;

    // trafilatura flattens <pre>/<code> blocks into bare inline <code> in its
    // content HTML; html2markdown then collapses them to single-line inline
    // code spans. Restore them as block-level <pre> before markdown conversion.
    result.content_html = restore_code_blocks(&result.content_html);

    let mut body = result.content_markdown();
    let title = choose_title(
        if result.metadata.title.is_empty() {
            None
        } else {
            Some(result.metadata.title.as_str())
        },
        &body,
    );

    // Strip leading H1 if it matches the selected title — we add our own in the markdown template
    if let Some(title_str) = title.as_deref() {
        let trimmed = body.trim_start();
        if let Some(rest) = trimmed.strip_prefix("# ") {
            let (first_line, remainder) = match rest.find('\n') {
                Some(pos) => (&rest[..pos], &rest[pos + 1..]),
                None => (rest, ""),
            };
            if first_line.trim().eq_ignore_ascii_case(title_str.trim()) {
                body = remainder.to_string();
            }
        }
    }

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

    let title = if let Some(title) = metadata_title {
        if !is_clearly_chrome_title(title) {
            Some(title.to_string())
        } else {
            first_meaningful_heading(body_markdown).or_else(|| metadata_title.map(str::to_string))
        }
    } else {
        first_meaningful_heading(body_markdown)
    };

    title.map(|t| {
        t.replace(['\u{2018}', '\u{2019}'], "'")
            .replace(['\u{201c}', '\u{201d}'], "\"")
    })
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
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
        == "keyboard shortcuts"
}

/// Strip markdown links `[text](url)` to just `text`, leaving fenced code blocks
/// verbatim. Code frequently contains `[x](y)`-shaped syntax (callbacks, regex,
/// config) that must not be mistaken for links once restored to a fenced block.
fn strip_markdown_links(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_fence = false;
    for line in input.split_inclusive('\n') {
        if is_fence_line(line) {
            in_fence = !in_fence;
            out.push_str(line);
        } else if in_fence {
            out.push_str(line);
        } else {
            out.push_str(&strip_md_links_text(line));
        }
    }
    out
}

/// A line that opens or closes a fenced code block: up to three leading spaces
/// followed by three or more backticks or tildes.
fn is_fence_line(line: &str) -> bool {
    let trimmed = line.trim_start_matches(' ');
    trimmed.starts_with("```") || trimmed.starts_with("~~~")
}

/// Strip markdown links `[text](url)` → text from a prose fragment.
/// Handles nested brackets conservatively.
fn strip_md_links_text(input: &str) -> String {
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

/// Wrap block-level `<code>` elements in `<pre>` so the markdown backend emits
/// fenced code blocks instead of collapsing them to single-line inline code.
///
/// trafilatura detects `<pre><code>` / `<pre lang>` blocks and re-emits them as
/// bare `<code>` in `content_html` (newlines intact). html2markdown renders a
/// bare `<code>` as *inline* code, which drops the newlines. A `<code>` is
/// treated as block-level when it spans multiple lines or is double-wrapped
/// (`<code><code>…`, the `<pre><code>` case); genuine inline code stays inline.
// ponytail: matches trafilatura's bare `<code>` output literally; if it ever
// emits `<code class=...>` those would pass through unchanged (rendered inline).
fn restore_code_blocks(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len());
    let mut cursor = 0usize;
    while let Some(rel) = lower[cursor..].find("<code>") {
        let start = cursor + rel;
        out.push_str(&html[cursor..start]);
        let inner_start = start + "<code>".len();
        match find_code_close(&lower, inner_start) {
            None => {
                out.push_str(&html[start..inner_start]);
                cursor = inner_start;
            }
            Some(close) => {
                let inner = &html[inner_start..close];
                if is_block_code(inner) {
                    out.push_str("<pre><code>");
                    out.push_str(inner);
                    out.push_str("</code></pre>");
                } else {
                    out.push_str(&html[start..close + "</code>".len()]);
                }
                cursor = close + "</code>".len();
            }
        }
    }
    out.push_str(&html[cursor..]);
    out
}

/// Byte index (in `lower`) of the `<` of the `</code>` that closes the `<code>`
/// opened before `from`, accounting for nested `<code>` (trafilatura emits
/// `<code><code>…</code></code>` for `<pre><code>`).
fn find_code_close(lower: &str, from: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut i = from;
    while i < lower.len() {
        if lower[i..].starts_with("<code>") {
            depth += 1;
            i += "<code>".len();
        } else if lower[i..].starts_with("</code>") {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
            i += "</code>".len();
        } else {
            i += lower[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        }
    }
    None
}

fn is_block_code(inner: &str) -> bool {
    inner.contains('\n') || inner.trim_start().starts_with("<code>")
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
