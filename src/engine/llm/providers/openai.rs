use async_trait::async_trait;
use serde_json::Value;

use super::{map_http_error, map_reqwest_error};
use crate::engine::llm::{
    sanitize_provider_error_message, FinishReason, LlmError, LlmProvider, LlmResponse, Message,
    NormalizedSchema, Role,
};

pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
}

impl OpenAiProvider {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn complete(
        &self,
        messages: &[Message],
        model: &str,
        max_tokens: u32,
        response_schema: Option<&NormalizedSchema>,
        _reasoning_disabled: bool,
    ) -> Result<LlmResponse, LlmError> {
        let api_messages: Vec<Value> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                };
                serde_json::json!({
                    "role": role,
                    "content": m.content,
                })
            })
            .collect();

        let mut body = serde_json::json!({
            "model": model,
            "messages": api_messages,
            "max_completion_tokens": max_tokens,
        });

        if let Some(schema) = response_schema {
            body["response_format"] = serde_json::json!({
                "type": "json_schema",
                "json_schema": {
                    "name": "response",
                    "schema": schema.0,
                    "strict": true,
                },
            });
        }

        let response = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| map_reqwest_error(&e))?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|e| LlmError::Parse(format!("failed to read response body: {}", e)))?;

        if !status.is_success() {
            return Err(map_http_error(status, &response_text));
        }

        let response_json: Value = serde_json::from_str(&response_text).map_err(|e| {
            let sanitized = sanitize_provider_error_message(&response_text);
            LlmError::Parse(format!("{}; body: {}", e, sanitized))
        })?;

        let choice = &response_json["choices"][0];
        let finish_reason_str = choice["finish_reason"].as_str().unwrap_or("stop");

        let finish_reason = match finish_reason_str {
            "stop" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "content_filter" => FinishReason::ContentFilter,
            other => FinishReason::Other(other.to_string()),
        };

        let content = choice["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        Ok(LlmResponse {
            content,
            finish_reason,
        })
    }
}
