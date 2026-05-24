use async_trait::async_trait;
use serde_json::Value;

use crate::engine::llm::{
    sanitize_provider_error_message, FinishReason, LlmError, LlmProvider, LlmResponse, Message,
    Role,
};

pub struct DeepSeekProvider {
    client: reqwest::Client,
    api_key: String,
}

impl DeepSeekProvider {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
        }
    }
}

#[async_trait]
impl LlmProvider for DeepSeekProvider {
    async fn complete(
        &self,
        messages: &[Message],
        model: &str,
        max_tokens: u32,
        response_schema: Option<&Value>,
        reasoning_disabled: bool,
    ) -> Result<LlmResponse, LlmError> {
        // 1. Convert messages to Deepseek chat format.
        let deepseek_messages: Vec<Value> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
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
            let schema_text = serde_json::to_string_pretty(schema)
                .map_err(|e| LlmError::Parse(format!("failed to serialize schema: {}", e)))?;
            let instruction = format!(
                "\n\nRespond with JSON matching this schema:\n```json\n{}\n```\n",
                schema_text
            );

            let mut cloned = deepseek_messages;

            // Find the first system message by role.
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
            deepseek_messages
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

        // 4. POST to DeepSeek API.
        let response = self
            .client
            .post("https://api.deepseek.com/chat/completions")
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

        // Validate structured output against schema when present.
        if let Some(schema) = response_schema {
            let content_val = &response_json["choices"][0]["message"]["content"];
            validate_structured_output(content_val, schema)?;
        }

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

// ── error mapping ─────────────────────────────────────────────────────────────

fn map_reqwest_error(error: &reqwest::Error) -> LlmError {
    let message = sanitize_provider_error_message(&error.to_string());
    LlmError::Network(message)
}

fn map_http_error(status: reqwest::StatusCode, body: &str) -> LlmError {
    let sanitized = sanitize_provider_error_message(body);
    let message = if sanitized.is_empty() {
        format!("HTTP {}", status.as_u16())
    } else {
        format!("HTTP {}: {}", status.as_u16(), sanitized)
    };

    if status.as_u16() == 429 {
        LlmError::RateLimited(message)
    } else if status.is_client_error() {
        LlmError::Api(message)
    } else {
        LlmError::Server(message)
    }
}

fn validate_structured_output(content_val: &Value, schema: &Value) -> Result<(), LlmError> {
    if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
        // Content may be a JSON string (parse it) or already an object.
        let parsed: Value = if content_val.is_string() {
            serde_json::from_str(content_val.as_str().unwrap_or_default()).unwrap_or(Value::Null)
        } else {
            content_val.clone()
        };
        for key in required {
            let key_str = key.as_str().unwrap_or_default();
            if parsed.get(key_str).is_none() {
                return Err(LlmError::Api(format!(
                    "DeepSeek response missing required field '{}' in structured output",
                    key_str
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../../../tests/engine_llm_deepseek_provider_tests.rs"]
mod tests;
