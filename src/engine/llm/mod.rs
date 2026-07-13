// Provider-agnostic LLM transport and protocol types.
//
// Owns provider calls, retry policy, structured responses, and tool-calling
// messages. The bounded turn loop lives in `engine::agent`; command-specific
// tools live in the CLI layer.

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

/// Error creating a provider: the custom provider needs a base URL from config.
#[derive(Debug)]
pub enum ProviderInitError {
    MissingBaseUrl,
    ConfigRead(String),
}

impl fmt::Display for ProviderInitError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderInitError::MissingBaseUrl => write!(
                f,
                "custom provider requires a base URL — run: bo config --provider custom --base-url <url>"
            ),
            ProviderInitError::ConfigRead(message) => {
                write!(f, "failed to read config for custom provider: {}", message)
            }
        }
    }
}

impl std::error::Error for ProviderInitError {}

/// Create the correct LlmProvider for a given provider with the API key.
///
/// For `Provider::Custom` the base URL is resolved from `~/.bo/config.json`,
/// mirroring how `auth::resolve_api_key` reads `~/.bo/auth.json`.
pub fn create_provider(
    provider: Provider,
    api_key: &str,
) -> Result<Box<dyn LlmProvider>, ProviderInitError> {
    match provider {
        Provider::OpenAI => Ok(Box::new(OpenAiProvider::new(api_key))),
        Provider::Deepseek => Ok(Box::new(OpenAiCompatProvider::deepseek(api_key))),
        Provider::Google => Ok(Box::new(GoogleProvider::new(api_key))),
        Provider::Zai => Ok(Box::new(OpenAiCompatProvider::zai(api_key))),
        Provider::Custom => {
            let base_url =
                crate::engine::config::custom_base_url(&crate::engine::config::config_path())
                    .map_err(|e| ProviderInitError::ConfigRead(e.to_string()))?
                    .ok_or(ProviderInitError::MissingBaseUrl)?;
            Ok(Box::new(OpenAiCompatProvider::custom(api_key, &base_url)))
        }
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
    /// Any OpenAI-compatible endpoint; base URL comes from config.
    #[serde(rename = "custom")]
    Custom,
}

impl Provider {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "openai" => Some(Provider::OpenAI),
            "deepseek" => Some(Provider::Deepseek),
            "google" => Some(Provider::Google),
            "zai" => Some(Provider::Zai),
            "custom" => Some(Provider::Custom),
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
            Provider::Custom => write!(f, "custom"),
        }
    }
}

pub const ALL_PROVIDERS: &[&str] = &["openai", "deepseek", "google", "zai", "custom"];

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

/// Retry-wrapped tool-calling completion. Mirrors `complete_with_policy`:
/// transient errors are retried with exponential backoff; the final error is
/// `RetryExhausted`. Tool schemas are passed through unchanged — the provider
/// serializes them into its native tool format. The agent loop bounds total
/// requests via its turn limit and this per-turn attempt count.
#[tracing::instrument(skip(provider, messages, tools), fields(model = %model))]
pub(crate) async fn complete_with_tools_with_policy(
    provider: &dyn LlmProvider,
    messages: &[AgentMessage],
    model: &str,
    max_tokens: u32,
    tools: &[ToolSchema],
    reasoning_disabled: bool,
    policy: LlmCallPolicy,
) -> Result<AgentResponse, LlmError> {
    if policy.max_attempts == 0 {
        return Err(LlmError::Api(
            "invalid LLM call policy: max_attempts must be at least 1".to_string(),
        ));
    }

    let mut last_error: Option<LlmError> = None;

    for attempt in 1..=policy.max_attempts {
        let result = tokio::time::timeout(
            policy.timeout,
            provider.complete_with_tools(messages, model, max_tokens, tools, reasoning_disabled),
        )
        .await;

        match result {
            Ok(Ok(response)) => return Ok(response),
            Ok(Err(error)) => {
                if !is_transient_error(&error) {
                    return Err(error);
                }
                tracing::warn!(attempt, %error, "transient tool-call failure");
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
    tracing::warn!(attempts = policy.max_attempts, %last_error, "tool-call retry exhausted");
    Err(LlmError::RetryExhausted {
        attempts: policy.max_attempts,
        last_error: Box::new(last_error),
    })
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

// ── Tool-calling extension ───────────────────────────────────────────────────
//
// Agent turn-loop message/response types. These extend LlmProvider for
// providers that support OpenAI-style function/tool calling. Providers that
// do not support tools leave the default `complete_with_tools`, which returns
// an explicit unsupported error so the agent loop never silently degrades.

/// A single tool call requested by the model.
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// Provider-assigned call id; must be echoed on the matching tool result.
    pub id: String,
    pub name: String,
    /// Raw JSON arguments string. Validated/deserialized into typed structs at
    /// the tool boundary — never trusted as-is.
    pub arguments: String,
}

/// A tool result fed back to the model, keyed by the call id it answers.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub tool_call_id: String,
    pub content: String,
}

/// Agent transcript message. Extends the simple `Message` with assistant
/// tool-call turns and tool-result turns required for multi-turn tool loops.
#[derive(Debug, Clone)]
pub enum AgentMessage {
    System(String),
    User(String),
    Assistant {
        content: Option<String>,
        /// DeepSeek returns `reasoning_content` in thinking mode and rejects
        /// the next request (HTTP 400) if it is omitted from the replayed
        /// assistant message. Stored here so the provider replays it verbatim.
        reasoning_content: Option<String>,
        tool_calls: Vec<ToolCall>,
    },
    Tool(ToolResult),
}

/// A provider response that may carry tool calls.
#[derive(Debug, Clone)]
pub struct AgentResponse {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: FinishReason,
    pub usage: Option<Usage>,
}

/// Token usage reported by the provider, captured for evaluation. Not enforced
/// as a budget in v0.0.10 — recorded only.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

/// A tool's name, description, and JSON-Schema parameters, ready for the
/// provider's native tool format.
#[derive(Debug, Clone)]
pub struct ToolSchema {
    pub name: String,
    pub description: String,
    pub parameters: Value,
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

    /// Complete with tool-calling support. Providers that do not support
    /// OpenAI-style tool calls leave this default, which fails explicitly so
    /// the agent loop surfaces an actionable error rather than silently
    /// degrading. The agent loop never calls `complete` for tool turns.
    async fn complete_with_tools(
        &self,
        _messages: &[AgentMessage],
        _model: &str,
        _max_tokens: u32,
        _tools: &[ToolSchema],
        _reasoning_disabled: bool,
    ) -> Result<AgentResponse, LlmError> {
        Err(LlmError::Api(
            "this provider does not support tool calls".to_string(),
        ))
    }
}

#[cfg(test)]
#[path = "../../tests/engine_llm_tests.rs"]
mod tests;
