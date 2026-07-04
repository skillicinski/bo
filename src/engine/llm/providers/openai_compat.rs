use async_trait::async_trait;
use serde_json::Value;

use super::{map_http_error, map_reqwest_error};
use crate::engine::llm::{
    sanitize_provider_error_message, FinishReason, LlmError, LlmProvider, LlmResponse, Message,
    NormalizedSchema, Role,
};

pub struct OpenAiCompatProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl OpenAiCompatProvider {
    pub fn deepseek(api_key: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            base_url: "https://api.deepseek.com/chat/completions".to_string(),
        }
    }

    pub fn zai(api_key: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            base_url: "https://api.z.ai/api/coding/paas/v4/chat/completions".to_string(),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiCompatProvider {
    async fn complete(
        &self,
        messages: &[Message],
        model: &str,
        max_tokens: u32,
        response_schema: Option<&NormalizedSchema>,
        reasoning_disabled: bool,
    ) -> Result<LlmResponse, LlmError> {
        // 1. Convert messages to chat format.
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

        let has_schema = response_schema.is_some();

        // 2. Embed response schema into the system message if requested.
        let final_messages = if let Some(schema) = response_schema {
            let schema_text = serde_json::to_string_pretty(&schema.0)
                .map_err(|e| LlmError::Parse(format!("failed to serialize schema: {}", e)))?;
            let instruction = format!(
                "\n\nRespond with JSON matching this schema:\n```json\n{}\n```\n",
                schema_text
            );

            let mut cloned = api_messages;

            let system_idx = cloned.iter().position(|m| m["role"] == "system");

            if let Some(idx) = system_idx {
                if let Some(content) = cloned[idx]["content"].as_str() {
                    let new_content = format!("{}{}", content, instruction);
                    cloned[idx] = serde_json::json!({
                        "role": "system",
                        "content": new_content,
                    });
                }
            } else {
                cloned.insert(
                    0,
                    serde_json::json!({
                        "role": "system",
                        "content": instruction.trim().to_string(),
                    }),
                );
            }

            cloned
        } else {
            api_messages
        };

        // 3. Build request body.
        let mut body = serde_json::json!({
            "model": model,
            "messages": final_messages,
            "max_tokens": max_tokens,
        });

        if has_schema {
            body["response_format"] = serde_json::json!({"type": "json_object"});
        }

        if reasoning_disabled {
            body["thinking"] = serde_json::json!({"type": "disabled"});
        }

        // 4. POST to API.
        let response = self
            .client
            .post(&self.base_url)
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

        // 5. Parse response JSON.
        let response_json: Value = serde_json::from_str(&response_text).map_err(|e| {
            let sanitized = sanitize_provider_error_message(&response_text);
            LlmError::Parse(format!("{}; body: {}", e, sanitized))
        })?;

        // 6. Extract content and finish_reason.
        let content = response_json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        let finish_reason_str = response_json["choices"][0]["finish_reason"]
            .as_str()
            .unwrap_or("stop");

        let finish_reason = match finish_reason_str {
            "stop" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "content_filter" => FinishReason::ContentFilter,
            other => FinishReason::Other(other.to_string()),
        };

        Ok(LlmResponse {
            content,
            finish_reason,
        })
    }
}

#[cfg(test)]
#[path = "../../../tests/engine_llm_openai_compat_provider_tests.rs"]
mod tests;
