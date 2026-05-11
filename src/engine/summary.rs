// Summary generation for collected leaves.
//
// Two paths:
//   generate_fallback(body) — deterministic, first ~200 words of body
//   generate_llm(body, title, provider, model, policy) — async LLM structured-output call
//
// The orchestrator `generate` tries LLM when OPENAI_API_KEY is set,
// falls back to deterministic only when no key/provider is configured.

use crate::engine::llm::{
    complete_with_policy, FinishReason, LlmCallPolicy, LlmError, LlmProvider, Message,
};
use serde::Deserialize;
use std::fmt;
use std::time::Duration;

// ── constants ────────────────────────────────────────────────────────────────

pub const SUMMARY_TARGET_WORDS: usize = 200;
pub const SUMMARY_INPUT_MAX_WORDS: usize = 4000;

const SUMMARY_LLM_POLICY: LlmCallPolicy = LlmCallPolicy {
    timeout: Duration::from_secs(30),
    max_attempts: 3,
    initial_backoff: Duration::from_millis(500),
};

const SUMMARY_SYSTEM_PROMPT: &str = "\
You are a document summarizer. Produce a single prose paragraph of approximately \
200 words that captures what the document is about: its key topics, main argument \
or thesis, and what makes it distinctive. The summary should be optimized for \
retrieval — a reader should be able to determine whether to read the full document \
based on your summary alone. Do not include meta-commentary like \"This document \
discusses...\" — write directly about the content.";

// ── errors ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum SummaryError {
    Llm(LlmError),
    Parse(String),
    Truncated,
    ContentFilter,
    Runtime(String),
}

impl fmt::Display for SummaryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SummaryError::Llm(error) => write!(f, "LLM summary failed: {}", error),
            SummaryError::Parse(message) => write!(f, "summary response parse error: {}", message),
            SummaryError::Truncated => write!(
                f,
                "summary output was truncated — try a model with larger output capacity"
            ),
            SummaryError::ContentFilter => write!(f, "summary was blocked by content filter"),
            SummaryError::Runtime(message) => write!(f, "summary runtime error: {}", message),
        }
    }
}

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
    policy: LlmCallPolicy,
) -> Result<String, SummaryError> {
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

    let response = complete_with_policy(provider, &messages, model, 512, Some(&schema), policy)
        .await
        .map_err(SummaryError::Llm)?;

    match response.finish_reason {
        FinishReason::Stop => {}
        FinishReason::Length => return Err(SummaryError::Truncated),
        FinishReason::ContentFilter => return Err(SummaryError::ContentFilter),
        FinishReason::Other(reason) => {
            return Err(SummaryError::Llm(LlmError::Api(format!(
                "unexpected finish reason: {}",
                reason
            ))));
        }
    }

    let parsed: SummaryResponse =
        serde_json::from_str(&response.content).map_err(|e| SummaryError::Parse(e.to_string()))?;

    if parsed.summary.trim().is_empty() {
        return Err(SummaryError::Parse(
            "LLM returned empty summary".to_string(),
        ));
    }

    Ok(parsed.summary)
}

// ── orchestrator ─────────────────────────────────────────────────────────────

/// Generate a summary for a leaf. Uses deterministic fallback when no provider
/// is configured. If a provider call is attempted, provider failures are errors.
pub fn generate(body: &str, title: Option<&str>, model: &str) -> Result<String, SummaryError> {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => return Ok(generate_fallback(body)),
    };

    eprintln!("summarizing...");

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| SummaryError::Runtime(format!("failed to create async runtime: {}", e)))?;

    let provider = crate::engine::llm::OpenAiProvider::new(&api_key);
    rt.block_on(generate_llm(
        body,
        title,
        &provider,
        model,
        SUMMARY_LLM_POLICY,
    ))
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "../tests/engine_summary_tests.rs"]
mod tests;
