use super::*;
use async_trait::async_trait;
use serde_json::Value;
use serial_test::serial;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use crate::engine::llm::{
    FinishReason, LlmCallPolicy, LlmError, LlmProvider, LlmResponse, Message,
};

struct EnvGuard {
    key: &'static str,
    original: Option<String>,
}

impl EnvGuard {
    fn set(key: &'static str, value: &str) -> Self {
        let original = std::env::var(key).ok();
        std::env::set_var(key, value);
        Self { key, original }
    }

    fn unset(key: &'static str) -> Self {
        let original = std::env::var(key).ok();
        std::env::remove_var(key);
        Self { key, original }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        match &self.original {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

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

struct SummaryFakeProvider {
    calls: AtomicUsize,
    fail_attempts: usize,
    finish_reason: FinishReason,
}

impl SummaryFakeProvider {
    fn new(fail_attempts: usize, finish_reason: FinishReason) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            fail_attempts,
            finish_reason,
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for SummaryFakeProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call <= self.fail_attempts {
            return Err(LlmError::Network("temporary failure".to_string()));
        }
        Ok(LlmResponse {
            content: r#"{"summary":"short useful summary"}"#.to_string(),
            finish_reason: self.finish_reason.clone(),
        })
    }
}

struct SummaryHangingProvider {
    calls: AtomicUsize,
}

impl SummaryHangingProvider {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for SummaryHangingProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(5)).await;
        Ok(LlmResponse {
            content: r#"{"summary":"late"}"#.to_string(),
            finish_reason: FinishReason::Stop,
        })
    }
}

fn short_summary_policy(max_attempts: usize) -> LlmCallPolicy {
    LlmCallPolicy {
        timeout: Duration::from_millis(20),
        max_attempts,
        initial_backoff: Duration::ZERO,
    }
}

#[test]
#[serial]
fn generate_without_api_key_returns_fallback() {
    let home = tempfile::TempDir::new().unwrap();
    let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
    let _api_key_guard = EnvGuard::unset("OPENAI_API_KEY");

    let summary = generate("one two three", Some("Title"), "gpt-4o").unwrap();

    assert_eq!(summary, "one two three");
}

#[test]
#[serial]
fn malformed_stored_auth_returns_non_secret_error() {
    let home = tempfile::TempDir::new().unwrap();
    let auth_path = home.path().join(".bo").join("auth.json");
    std::fs::create_dir_all(auth_path.parent().unwrap()).unwrap();
    std::fs::write(&auth_path, "sk-summary-leak-marker").unwrap();
    let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
    let _api_key_guard = EnvGuard::unset("OPENAI_API_KEY");

    let error = generate("one two three", Some("Title"), "gpt-4o").unwrap_err();
    let message = error.to_string();

    assert!(matches!(error, SummaryError::Runtime(_)));
    assert!(!message.contains("sk-summary-leak-marker"));
}

#[tokio::test(flavor = "current_thread")]
async fn attempted_summary_retries_transient_failure_and_succeeds() {
    let provider = SummaryFakeProvider::new(1, FinishReason::Stop);

    let summary = generate_llm(
        "Body text for summary generation.",
        Some("Title"),
        &provider,
        "gpt-4o",
        short_summary_policy(3),
    )
    .await
    .unwrap();

    assert_eq!(provider.calls(), 2);
    assert_eq!(summary, "short useful summary");
}

#[tokio::test(flavor = "current_thread")]
async fn attempted_summary_timeout_returns_error_not_fallback() {
    let provider = SummaryHangingProvider::new();

    let err = generate_llm(
        "Body text for summary generation.",
        Some("Title"),
        &provider,
        "gpt-4o",
        short_summary_policy(1),
    )
    .await
    .unwrap_err();

    assert_eq!(provider.calls(), 1);
    assert!(matches!(
        err,
        SummaryError::Llm(LlmError::RetryExhausted { .. })
    ));
}

#[tokio::test(flavor = "current_thread")]
async fn attempted_summary_length_finish_reason_fails() {
    let provider = SummaryFakeProvider::new(0, FinishReason::Length);

    let err = generate_llm(
        "Body text for summary generation.",
        Some("Title"),
        &provider,
        "gpt-4o",
        short_summary_policy(1),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, SummaryError::Truncated));
}

#[tokio::test(flavor = "current_thread")]
async fn attempted_summary_content_filter_finish_reason_fails() {
    let provider = SummaryFakeProvider::new(0, FinishReason::ContentFilter);

    let err = generate_llm(
        "Body text for summary generation.",
        Some("Title"),
        &provider,
        "gpt-4o",
        short_summary_policy(1),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, SummaryError::ContentFilter));
}
