// Provider-agnostic LLM calling infrastructure.
//
// No agent or tool-calling concepts. This module provides a trait for sending
// messages to an LLM and receiving structured responses.

pub mod providers;

pub use providers::OpenAiProvider;

use async_trait::async_trait;
use serde_json::Value;
use std::fmt;

// ── public types ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum LlmError {
    Network(String),
    Api(String),
    Parse(String),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::Network(s) => write!(f, "network error: {}", s),
            LlmError::Api(s) => write!(f, "API error: {}", s),
            LlmError::Parse(s) => write!(f, "response parse error: {}", s),
        }
    }
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
