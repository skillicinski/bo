// Provider-agnostic LLM calling infrastructure.
//
// No agent or tool-calling concepts. This module provides a trait for sending
// messages to an LLM and receiving structured responses.

pub mod model;
pub mod models;
pub mod providers;

pub use model::{Model, UnsupportedModel};
pub use models::context_window_tokens;
pub use providers::{GoogleProvider, OpenAiCompatProvider, OpenAiProvider};

/// Sanitize a provider error message by redacting API key fragments.
pub(crate) fn sanitize_provider_error_message(message: &str) -> String {
    message
        .split_whitespace()
        .map(|token| {
            if token.contains("sk-") || token.contains("AIzaSy") {
                "<redacted>".to_string()
            } else {
                token.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Create the correct LlmProvider for a given provider with the API key.
pub fn create_provider(provider: Provider, api_key: &str) -> Box<dyn LlmProvider> {
    match provider {
        Provider::OpenAI => Box::new(OpenAiProvider::new(api_key)),
        Provider::Deepseek => Box::new(OpenAiCompatProvider::deepseek(api_key)),
        Provider::Google => Box::new(GoogleProvider::new(api_key)),
        Provider::Zai => Box::new(OpenAiCompatProvider::zai(api_key)),
    }
}

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::sync::OnceLock;
use std::time::Duration;

// ── shared async executor ────────────────────────────────────────────────────

/// One current-thread tokio runtime shared by all blocking LLM calls.
/// Collect, query, and compile all call into the same runtime — avoid three
/// different instantiation patterns and the per-call builder overhead.
///
/// Panics if the runtime cannot be built (fatal, process-wide).
pub fn blocking_runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build shared tokio runtime")
    })
}

// ── Provider enum ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Provider {
    #[serde(rename = "openai")]
    OpenAI,
    #[serde(rename = "deepseek")]
    Deepseek,
    #[serde(rename = "google")]
    Google,
    #[serde(rename = "zai")]
    Zai,
}

impl Provider {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "openai" => Some(Provider::OpenAI),
            "deepseek" => Some(Provider::Deepseek),
            "google" => Some(Provider::Google),
            "zai" => Some(Provider::Zai),
            _ => None,
        }
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Provider::OpenAI => write!(f, "openai"),
            Provider::Deepseek => write!(f, "deepseek"),
            Provider::Google => write!(f, "google"),
            Provider::Zai => write!(f, "zai"),
        }
    }
}

pub const ALL_PROVIDERS: &[&str] = &["openai", "deepseek", "google", "zai"];

// ── public types ──────────────────────────────────────────────────────────────

/// A response schema that has been normalized into a provider's native dialect.
///
/// Only `LlmProvider::map_response_schema` can produce this value.
/// Passing `Option<&NormalizedSchema>` to `complete()` is a compile-time
/// guarantee that normalization happened — you cannot forget.
#[derive(Debug, Clone)]
pub struct NormalizedSchema(pub(crate) Value);

#[derive(Debug)]
pub enum LlmError {
    Network(String),
    RateLimited(String),
    Server(String),
    Api(String),
    Parse(String),
    Timeout {
        timeout: Duration,
    },
    RetryExhausted {
        attempts: usize,
        last_error: Box<LlmError>,
    },
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::Network(s) => write!(f, "network error: {}", s),
            LlmError::RateLimited(s) => write!(f, "rate limited: {}", s),
            LlmError::Server(s) => write!(f, "server error: {}", s),
            LlmError::Api(s) => write!(f, "API error: {}", s),
            LlmError::Parse(s) => write!(f, "response parse error: {}", s),
            LlmError::Timeout { timeout } => {
                write!(f, "LLM request timed out after {}s", timeout.as_secs_f64())
            }
            LlmError::RetryExhausted {
                attempts,
                last_error,
            } => write!(
                f,
                "LLM request failed after {} attempts: {}",
                attempts, last_error
            ),
        }
    }
}

// ── Call policy ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct LlmCallPolicy {
    pub timeout: Duration,
    pub max_attempts: usize,
    pub initial_backoff: Duration,
}

#[tracing::instrument(skip(provider, messages, response_schema, policy), fields(model = %model))]
pub async fn complete_with_policy(
    provider: &dyn LlmProvider,
    messages: &[Message],
    model: &str,
    max_tokens: u32,
    response_schema: Option<&Value>,
    reasoning_disabled: bool,
    policy: LlmCallPolicy,
) -> Result<LlmResponse, LlmError> {
    if policy.max_attempts == 0 {
        return Err(LlmError::Api(
            "invalid LLM call policy: max_attempts must be at least 1".to_string(),
        ));
    }

    // Normalize the schema into the provider's native dialect once,
    // before the retry loop. A schema the provider cannot satisfy
    // fails fast here rather than after N retries.
    let normalized_schema = match response_schema {
        Some(schema) => Some(provider.map_response_schema(schema)?),
        None => None,
    };

    let mut last_error: Option<LlmError> = None;

    for attempt in 1..=policy.max_attempts {
        let result = tokio::time::timeout(
            policy.timeout,
            provider.complete(
                messages,
                model,
                max_tokens,
                normalized_schema.as_ref(),
                reasoning_disabled,
            ),
        )
        .await;

        match result {
            Ok(Ok(response)) => return Ok(response),
            Ok(Err(error)) => {
                if !is_transient_error(&error) {
                    return Err(error);
                }
                tracing::warn!(attempt, %error, "transient LLM failure");
                last_error = Some(error);
            }
            Err(_) => {
                last_error = Some(LlmError::Timeout {
                    timeout: policy.timeout,
                });
            }
        }

        if attempt < policy.max_attempts {
            let delay = retry_delay(policy.initial_backoff, attempt);
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
        }
    }

    let last_error = last_error
        .unwrap_or_else(|| LlmError::Api("LLM request failed without an error".to_string()));
    tracing::warn!(attempts = policy.max_attempts, %last_error, "LLM retry exhausted");
    Err(LlmError::RetryExhausted {
        attempts: policy.max_attempts,
        last_error: Box::new(last_error),
    })
}

pub fn is_transient_error(error: &LlmError) -> bool {
    matches!(
        error,
        LlmError::Network(_)
            | LlmError::RateLimited(_)
            | LlmError::Server(_)
            | LlmError::Timeout { .. }
    )
}

fn retry_delay(initial_backoff: Duration, completed_attempt: usize) -> Duration {
    let multiplier = if completed_attempt > u32::BITS as usize {
        u32::MAX
    } else {
        1u32 << completed_attempt.saturating_sub(1)
    };
    initial_backoff.saturating_mul(multiplier)
}

// ── Message types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
    User,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
}

// ── Response types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Stop,
    Length,
    ContentFilter,
    Other(String),
}

#[derive(Debug)]
pub struct LlmResponse {
    pub content: String,
    pub finish_reason: FinishReason,
}

// ── LlmProvider trait ─────────────────────────────────────────────────────────

/// An LLM backend that can produce structured responses.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Transform a response schema into the provider's native dialect.
    ///
    /// Default: identity (pass-through). Override if the provider's schema
    /// vocabulary differs from the canonical JSON Schema dialect (e.g. Gemini
    /// does not accept `additionalProperties`).
    ///
    /// Return `Err` if the schema uses constructs the provider cannot satisfy —
    /// the caller must not silently degrade. This is called once per request
    /// by `complete_with_policy`, before the retry loop.
    fn map_response_schema(&self, schema: &Value) -> Result<NormalizedSchema, LlmError> {
        // ponytail: default identity – most providers accept canonical JSON Schema as-is
        Ok(NormalizedSchema(schema.clone()))
    }

    async fn complete(
        &self,
        messages: &[Message],
        model: &str,
        max_tokens: u32,
        response_schema: Option<&NormalizedSchema>,
        reasoning_disabled: bool,
    ) -> Result<LlmResponse, LlmError>;
}

#[cfg(test)]
#[path = "../../tests/engine_llm_tests.rs"]
mod tests;
