// Summary generation for collected leaves.
//
// Two paths:
//   generate_fallback(body) — deterministic, first ~200 words of body
//   generate_llm(body, title, provider, model) — async LLM structured-output call
//
// The orchestrator `generate` tries LLM when OPENAI_API_KEY is set,
// falls back to deterministic on missing key or any error.

use crate::engine::llm::{LlmError, LlmProvider, Message};
use serde::Deserialize;
use tracing::warn;

// ── constants ────────────────────────────────────────────────────────────────

pub const SUMMARY_TARGET_WORDS: usize = 200;
pub const SUMMARY_INPUT_MAX_WORDS: usize = 4000;

const SUMMARY_SYSTEM_PROMPT: &str = "\
You are a document summarizer. Produce a single prose paragraph of approximately \
200 words that captures what the document is about: its key topics, main argument \
or thesis, and what makes it distinctive. The summary should be optimized for \
retrieval — a reader should be able to determine whether to read the full document \
based on your summary alone. Do not include meta-commentary like \"This document \
discusses...\" — write directly about the content.";

// ── deterministic fallback ───────────────────────────────────────────────────

/// Generate a summary by extracting the first ~200 words of body content.
/// Truncates at a word boundary. Returns empty string for empty body.
pub fn generate_fallback(body: &str) -> String {
    let words: Vec<&str> = body.split_whitespace().collect();
    if words.len() <= SUMMARY_TARGET_WORDS {
        words.join(" ")
    } else {
        words[..SUMMARY_TARGET_WORDS].join(" ")
    }
}

/// Truncate body to the first `max_words` words for LLM input.
pub fn truncate_body(body: &str, max_words: usize) -> String {
    let words: Vec<&str> = body.split_whitespace().collect();
    if words.len() <= max_words {
        words.join(" ")
    } else {
        words[..max_words].join(" ")
    }
}

// ── LLM-powered summary ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SummaryResponse {
    summary: String,
}

/// Generate a summary via a single structured-output LLM call.
pub async fn generate_llm(
    body: &str,
    title: Option<&str>,
    provider: &dyn LlmProvider,
    model: &str,
) -> Result<String, LlmError> {
    let truncated_body = truncate_body(body, SUMMARY_INPUT_MAX_WORDS);

    let user_message = match title {
        Some(t) => format!(
            "<title>{}</title>\n<document>\n{}\n</document>",
            t, truncated_body
        ),
        None => format!("<document>\n{}\n</document>", truncated_body),
    };

    let messages = vec![
        Message::system(SUMMARY_SYSTEM_PROMPT),
        Message::user(user_message),
    ];

    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string" }
        },
        "required": ["summary"],
        "additionalProperties": false
    });

    let response = provider
        .complete(&messages, model, 512, Some(&schema))
        .await?;

    let parsed: SummaryResponse =
        serde_json::from_str(&response.content).map_err(|e| LlmError::Parse(e.to_string()))?;

    if parsed.summary.trim().is_empty() {
        return Err(LlmError::Parse("LLM returned empty summary".to_string()));
    }

    Ok(parsed.summary)
}

// ── orchestrator ─────────────────────────────────────────────────────────────

/// Generate a summary for a leaf. Tries LLM if OPENAI_API_KEY is set,
/// falls back to deterministic extraction on missing key or any error.
pub fn generate(body: &str, title: Option<&str>, model: &str) -> String {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return generate_fallback(body),
    };

    eprintln!("summarizing...");

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            warn!("failed to create async runtime for summary: {}", e);
            return generate_fallback(body);
        }
    };

    let provider = crate::engine::llm::OpenAiProvider::new(&api_key);

    match rt.block_on(generate_llm(body, title, &provider, model)) {
        Ok(summary) => summary,
        Err(e) => {
            warn!("LLM summary failed, using fallback: {}", e);
            generate_fallback(body)
        }
    }
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_empty_body() {
        assert_eq!(generate_fallback(""), "");
    }

    #[test]
    fn fallback_short_body_returned_as_is() {
        let body = "This is a short body with only a few words.";
        assert_eq!(generate_fallback(body), body);
    }

    #[test]
    fn fallback_truncates_at_200_words() {
        let words: Vec<String> = (0..300).map(|i| format!("word{}", i)).collect();
        let body = words.join(" ");
        let result = generate_fallback(&body);
        assert_eq!(result.split_whitespace().count(), 200);
        assert!(result.starts_with("word0 word1"));
        assert!(result.ends_with("word199"));
    }

    #[test]
    fn fallback_normalizes_whitespace() {
        let body = "hello   world\n\nnew  paragraph\there";
        let result = generate_fallback(body);
        assert_eq!(result, "hello world new paragraph here");
    }

    #[test]
    fn fallback_exactly_200_words() {
        let words: Vec<String> = (0..200).map(|i| format!("w{}", i)).collect();
        let body = words.join(" ");
        let result = generate_fallback(&body);
        assert_eq!(result.split_whitespace().count(), 200);
    }

    #[test]
    fn truncate_body_short_input_unchanged() {
        let body = "short text here";
        assert_eq!(truncate_body(body, 4000), "short text here");
    }

    #[test]
    fn truncate_body_long_input_cut() {
        let words: Vec<String> = (0..5000).map(|i| format!("w{}", i)).collect();
        let body = words.join(" ");
        let result = truncate_body(&body, 4000);
        assert_eq!(result.split_whitespace().count(), 4000);
    }
}
