use super::*;
use async_trait::async_trait;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

struct TransientThenSuccessProvider {
    fail_attempts: usize,
    calls: AtomicUsize,
}

impl TransientThenSuccessProvider {
    fn new(fail_attempts: usize) -> Self {
        Self {
            fail_attempts,
            calls: AtomicUsize::new(0),
        }
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for TransientThenSuccessProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&NormalizedSchema>,
        _reasoning_disabled: bool,
    ) -> Result<LlmResponse, LlmError> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
        if call <= self.fail_attempts {
            return Err(LlmError::Network("temporary network failure".to_string()));
        }
        Ok(LlmResponse {
            content: "ok".to_string(),
            finish_reason: FinishReason::Stop,
        })
    }
}

struct PermanentFailureProvider {
    calls: AtomicUsize,
}

impl PermanentFailureProvider {
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
impl LlmProvider for PermanentFailureProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&NormalizedSchema>,
        _reasoning_disabled: bool,
    ) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Err(LlmError::Parse("invalid response".to_string()))
    }
}

struct HangingProvider {
    calls: AtomicUsize,
}

impl HangingProvider {
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
impl LlmProvider for HangingProvider {
    async fn complete(
        &self,
        _messages: &[Message],
        _model: &str,
        _max_tokens: u32,
        _response_schema: Option<&NormalizedSchema>,
        _reasoning_disabled: bool,
    ) -> Result<LlmResponse, LlmError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(5)).await;
        Ok(LlmResponse {
            content: "late".to_string(),
            finish_reason: FinishReason::Stop,
        })
    }
}

fn short_retry_policy(max_attempts: usize) -> LlmCallPolicy {
    LlmCallPolicy {
        timeout: Duration::from_millis(20),
        max_attempts,
        initial_backoff: Duration::ZERO,
    }
}

#[test]
fn model_catalog_includes_gpt4_1_mini() {
    assert!(models::is_supported_model(Provider::OpenAI, "gpt-4.1-mini"));
}

#[test]
fn model_catalog_lists_openai_models() {
    let models = models::models_for(Provider::OpenAI);
    let ids: Vec<&str> = models.iter().map(|m| m.id).collect();
    assert!(ids.contains(&"gpt-4o"));
    assert!(ids.contains(&"gpt-4.1-mini"));
}

#[test]
fn model_catalog_lists_deepseek_models() {
    let models = models::models_for(Provider::Deepseek);
    let ids: Vec<&str> = models.iter().map(|m| m.id).collect();
    assert!(ids.contains(&"deepseek-v4-flash"));
    assert!(ids.contains(&"deepseek-v4-pro"));
}

#[test]
fn context_window_lookup_returns_known_windows() {
    assert_eq!(
        context_window_tokens(Provider::OpenAI, "gpt-4o"),
        Some(128_000)
    );
    assert_eq!(
        context_window_tokens(Provider::OpenAI, "gpt-4.1-mini"),
        Some(1_000_000)
    );
}

#[test]
fn deepseek_context_window() {
    assert_eq!(
        context_window_tokens(Provider::Deepseek, "deepseek-v4-pro"),
        Some(1_000_000)
    );
}

#[test]
fn model_catalog_rejects_unsupported_models() {
    assert!(!models::is_supported_model(
        Provider::OpenAI,
        "unknown-model"
    ));
    assert_eq!(
        context_window_tokens(Provider::OpenAI, "unknown-model"),
        None
    );
}

#[tokio::test(flavor = "current_thread")]
async fn complete_with_policy_succeeds_after_one_transient_failure() {
    let provider = TransientThenSuccessProvider::new(1);

    let response = complete_with_policy(
        &provider,
        &[],
        "gpt-4o",
        10,
        None,
        false,
        short_retry_policy(3),
    )
    .await
    .unwrap();

    assert_eq!(response.content, "ok");
    assert_eq!(provider.calls(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn complete_with_policy_exhausts_after_three_transient_attempts() {
    let provider = TransientThenSuccessProvider::new(usize::MAX);

    let err = complete_with_policy(
        &provider,
        &[],
        "gpt-4o",
        10,
        None,
        false,
        short_retry_policy(3),
    )
    .await
    .unwrap_err();

    assert_eq!(provider.calls(), 3);
    assert!(matches!(err, LlmError::RetryExhausted { attempts: 3, .. }));
}

#[tokio::test(flavor = "current_thread")]
async fn complete_with_policy_times_out_hanging_provider() {
    let provider = HangingProvider::new();

    let err = complete_with_policy(
        &provider,
        &[],
        "gpt-4o",
        10,
        None,
        false,
        short_retry_policy(1),
    )
    .await
    .unwrap_err();

    assert_eq!(provider.calls(), 1);
    match err {
        LlmError::RetryExhausted {
            attempts: 1,
            last_error,
        } => {
            assert!(matches!(*last_error, LlmError::Timeout { .. }));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn complete_with_policy_does_not_retry_permanent_error() {
    let provider = PermanentFailureProvider::new();

    let err = complete_with_policy(
        &provider,
        &[],
        "gpt-4o",
        10,
        None,
        false,
        short_retry_policy(3),
    )
    .await
    .unwrap_err();

    assert_eq!(provider.calls(), 1);
    assert!(matches!(err, LlmError::Parse(_)));
}
