// Collect stage: acquire and normalize web/YouTube/note content into a
// `ComputedLeaf` — everything needed to write a leaf, with no disk I/O.
// Safe to call from multiple worker threads concurrently.

use std::fs;
use std::sync::Arc;

use crate::adapters::youtube::{self, YoutubeError, YoutubeUrlMatch};
use crate::engine::auth;
use crate::engine::pending;
use crate::engine::quality;
use crate::engine::{extract, fetch, summary};

use super::{CollectError, NoteError};

// ── SummaryProvider — resolve once per invocation ────────────────────────────

/// Resolved summary provider, shared across all worker threads. Auth and
/// provider construction happen once per collect invocation; failure falls
/// back to deterministic summaries.
pub(super) type Summarize = Arc<dyn Fn(&str, Option<&str>) -> String + Send + Sync>;

#[derive(Clone)]
pub(super) struct SummaryProvider {
    summarize: Summarize,
}

impl SummaryProvider {
    pub(super) fn resolve(
        provider: crate::engine::llm::Provider,
        model: String,
        base_url: Option<&str>,
    ) -> Self {
        let resolved = (|| {
            let api_key = auth::resolve_api_key(provider).ok()?;
            crate::engine::llm::create_provider(provider, &api_key, base_url).ok()
        })();
        match resolved {
            Some(p) => {
                let p: Arc<dyn crate::engine::llm::LlmProvider> = Arc::from(p);
                Self {
                    summarize: Arc::new(move |body, title| {
                        crate::engine::llm::blocking_runtime().block_on(
                            summary::generate_llm_or_fallback(
                                body,
                                title,
                                p.as_ref(),
                                &model,
                                summary::SUMMARY_LLM_POLICY,
                            ),
                        )
                    }),
                }
            }
            None => Self {
                summarize: Arc::new(|body, _| summary::generate_fallback(body)),
            },
        }
    }

    #[cfg(test)]
    pub(super) fn fallback() -> Self {
        Self {
            summarize: Arc::new(|body, _| summary::generate_fallback(body)),
        }
    }

    pub(super) fn summarize(&self, body: &str, title: Option<&str>) -> String {
        (self.summarize)(body, title)
    }
}

// ── data for parallel batch compute ──────────────────────────────────────────

/// Output of the compute phase: everything needed to write a leaf, but no
/// disk I/O performed yet. Safe to produce from multiple threads concurrently.
#[derive(Debug)]
pub(super) struct ComputedLeaf {
    pub(super) url: String,
    pub(super) title: Option<String>,
    pub(super) body_markdown: String,
    pub(super) summary_text: String,
    /// Frontmatter-strip warning for notes; `None` for fetched URLs.
    pub(super) note_warning: Option<String>,
}

// ── pipeline ─────────────────────────────────────────────────────────────────

/// Compute-only: fetch, extract, quality-check, and summarize a URL.
/// Returns the data needed to write a leaf without touching the manifest or
/// output directory. Safe to call from multiple threads.
pub(super) fn compute_leaf(
    url: &str,
    summary: &SummaryProvider,
) -> Result<ComputedLeaf, CollectError> {
    // YouTube path — fetch transcript, summarize, return.
    match youtube::classify_url(url) {
        YoutubeUrlMatch::Supported(supported) => {
            let transcript = youtube::collect_transcript(&supported)?;
            let summary_text =
                summary.summarize(&transcript.body_markdown, Some(&transcript.title));
            return Ok(ComputedLeaf {
                url: transcript.url,
                title: Some(transcript.title),
                body_markdown: transcript.body_markdown,
                summary_text,
                note_warning: None,
            });
        }
        YoutubeUrlMatch::Unsupported { url, reason } => {
            return Err(YoutubeError::UnsupportedUrl { url, reason }.into());
        }
        YoutubeUrlMatch::NotYoutube => {}
    }

    // Fetch, classify HTTP errors, extract, quality-check, summarize.
    let fetched = match fetch::fetch_url(url) {
        Ok(fetched) => fetched,
        Err(fetch::FetchError::HttpStatus(status, message)) => {
            if let Some(reason) = quality::classify_http_status(status) {
                return Err(CollectError::Rejected {
                    url: url.to_string(),
                    reason,
                });
            }
            return Err(fetch::FetchError::HttpStatus(status, message).into());
        }
        Err(e) => return Err(e.into()),
    };

    compute_leaf_from_html(&fetched.url, &fetched.html, |b, t| summary.summarize(b, t))
}

/// Test seam: the HTML core of `compute_leaf` without network I/O.
/// Injects pre-fetched HTML and a custom summarize closure.
pub(super) fn compute_leaf_from_html<F>(
    url: &str,
    html: &str,
    summarize: F,
) -> Result<ComputedLeaf, CollectError>
where
    F: FnOnce(&str, Option<&str>) -> String,
{
    if let Some(reason) = quality::classify_html(html) {
        return Err(CollectError::Rejected {
            url: url.to_string(),
            reason,
        });
    }

    let content = extract::extract_content(html)?;

    if let Some(reason) =
        quality::classify_extracted(content.title.as_deref(), &content.body_markdown)
    {
        return Err(CollectError::Rejected {
            url: url.to_string(),
            reason,
        });
    }

    let summary_text = summarize(&content.body_markdown, content.title.as_deref());

    Ok(ComputedLeaf {
        url: url.to_string(),
        title: content.title,
        body_markdown: content.body_markdown,
        summary_text,
        note_warning: None,
    })
}

/// Compute a note from a local `.md` file: read, strip user frontmatter,
/// extract a title from the leading H1, and derive a content-addressed source
/// URL. No fetch, no extract, no summary — notes store `summary: None`.
///
/// # ponytail: `bo://note/<sha256[:16]>` is a documented overload of `url` as
/// source-id for source-less leaves. Upgrade to `Option<Url>` in the v0.1.0
/// restructure if notes prove out.
pub(super) fn compute_leaf_note(path: &str) -> Result<ComputedLeaf, CollectError> {
    let raw = fs::read_to_string(path).map_err(|error| {
        CollectError::Note(NoteError::Read {
            path: path.to_string(),
            error,
        })
    })?;
    let (body_after_frontmatter, note_warning) = strip_user_frontmatter(path, &raw)?;
    if body_after_frontmatter.trim().is_empty() {
        return Err(CollectError::Note(NoteError::Empty {
            path: path.to_string(),
        }));
    }
    // Hash the frontmatter-stripped body (including the H1 line) so identical
    // notes dedup and edits mint a fresh leaf. sha256[:16] = 64 bits.
    let url = format!(
        "bo://note/{}",
        &pending::content_hash(body_after_frontmatter.as_bytes())[..16]
    );
    let (title, body_markdown) = extract_note_title(&body_after_frontmatter);
    Ok(ComputedLeaf {
        url,
        title,
        body_markdown,
        summary_text: String::new(),
        note_warning,
    })
}

/// Strip a leading YAML frontmatter block from a note. Returns the body and
/// an optional warning when non-empty user frontmatter was removed — leaf
/// frontmatter is bo-owned, so the shaping is always visible.
fn strip_user_frontmatter(path: &str, raw: &str) -> Result<(String, Option<String>), CollectError> {
    use crate::domain::frontmatter::{parse, FrontmatterError};
    match parse(raw) {
        Ok((mapping, body)) => {
            let warning = if mapping.is_empty() {
                None
            } else {
                Some(format!("stripped user frontmatter from {}", path))
            };
            Ok((body, warning))
        }
        Err(FrontmatterError::Missing) => Ok((raw.to_string(), None)),
        Err(_) => Err(CollectError::Note(NoteError::MalformedFrontmatter {
            path: path.to_string(),
        })),
    }
}

/// Pull a title from a leading `# ` heading and strip it from the body,
/// mirroring fetched-page handling (`format_content` re-prepends `# title`).
fn extract_note_title(body: &str) -> (Option<String>, String) {
    let after_blank_lines = body.trim_start_matches('\n');
    let Some(rest) = after_blank_lines.strip_prefix("# ") else {
        return (None, body.to_string());
    };
    let (heading, remainder) = match rest.find('\n') {
        Some(idx) => (
            &rest[..idx],
            rest[idx + 1..].trim_start_matches('\n').to_string(),
        ),
        None => (rest, String::new()),
    };
    let title = heading.trim();
    if title.is_empty() {
        return (None, body.to_string());
    }
    (Some(title.to_string()), remainder)
}

#[cfg(test)]
#[path = "../../tests/cli_collect_compute_tests.rs"]
mod tests;
