// Provider-agnostic LLM calling infrastructure.
//
// No agent or tool-calling concepts. This module provides a trait for sending
// messages to an LLM and receiving structured responses.

pub mod providers;

pub use providers::OpenAiProvider;

use async_trait::async_trait;
use serde_json::Value;
use std::fmt;
use std::time::Duration;

// ── public types ──────────────────────────────────────────────────────────────

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

// ── Model metadata ───────────────────────────────────────────────────────────

struct ModelContextWindow {
    model: &'static str,
    context_tokens: usize,
}

const MODEL_CONTEXT_WINDOWS: &[ModelContextWindow] = &[
    ModelContextWindow {
        model: "gpt-4o",
        context_tokens: 128_000,
    },
    ModelContextWindow {
        model: "gpt-4o-mini",
        context_tokens: 128_000,
    },
    ModelContextWindow {
        model: "gpt-4.1",
        context_tokens: 1_000_000,
    },
    ModelContextWindow {
        model: "gpt-4.1-mini",
        context_tokens: 1_000_000,
    },
    ModelContextWindow {
        model: "gpt-4.1-nano",
        context_tokens: 1_000_000,
    },
];

pub fn context_window_tokens(model: &str) -> Option<usize> {
    let model = model.trim();
    MODEL_CONTEXT_WINDOWS
        .iter()
        .find(|entry| entry.model == model)
        .map(|entry| entry.context_tokens)
}

// ── Call policy ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy)]
pub struct LlmCallPolicy {
    pub timeout: Duration,
    pub max_attempts: usize,
    pub initial_backoff: Duration,
}

pub async fn complete_with_policy(
    provider: &dyn LlmProvider,
    messages: &[Message],
    model: &str,
    max_tokens: u32,
    response_schema: Option<&Value>,
    policy: LlmCallPolicy,
) -> Result<LlmResponse, LlmError> {
    if policy.max_attempts == 0 {
        return Err(LlmError::Api(
            "invalid LLM call policy: max_attempts must be at least 1".to_string(),
        ));
    }

    let mut last_error: Option<LlmError> = None;

    for attempt in 1..=policy.max_attempts {
        let result = tokio::time::timeout(
            policy.timeout,
            provider.complete(messages, model, max_tokens, response_schema),
        )
        .await;

        match result {
            Ok(Ok(response)) => return Ok(response),
            Ok(Err(error)) => {
                if !is_transient_error(&error) {
                    return Err(error);
                }
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

    Err(LlmError::RetryExhausted {
        attempts: policy.max_attempts,
        last_error: Box::new(
            last_error.unwrap_or_else(|| {
                LlmError::Api("LLM request failed without an error".to_string())
            }),
        ),
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
    Assistant,
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
    async fn complete(
        &self,
        messages: &[Message],
        model: &str,
        max_tokens: u32,
        response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError>;
}

#[cfg(test)]
#[path = "../../tests/engine_llm_tests.rs"]
mod tests;
