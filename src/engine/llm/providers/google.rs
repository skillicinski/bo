use async_trait::async_trait;
use serde_json::Value;

use crate::engine::llm::{
    sanitize_provider_error_message, FinishReason, LlmError, LlmProvider, LlmResponse, Message,
    Role,
};

pub struct GoogleProvider {
    client: reqwest::Client,
    api_key: String,
}

impl GoogleProvider {
    pub fn new(api_key: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
        }
    }
}

#[async_trait]
impl LlmProvider for GoogleProvider {
    async fn complete(
        &self,
        messages: &[Message],
        model: &str,
        max_tokens: u32,
        response_schema: Option<&Value>,
        reasoning_disabled: bool,
    ) -> Result<LlmResponse, LlmError> {
        // 1. Separate system messages from conversation turns.
        let mut system_parts: Vec<Value> = Vec::new();
        let mut contents: Vec<Value> = Vec::new();

        for m in messages {
            match m.role {
                Role::System => {
                    system_parts.push(serde_json::json!({"text": m.content}));
                }
                Role::User => {
                    contents.push(serde_json::json!({
                        "role": "user",
                        "parts": [{"text": m.content}]
                    }));
                }
                Role::Assistant => {
                    // The Gemini API natively uses "model" for assistant turns.
                    // This arm exists for forward-compatibility; bo does not
                    // currently emit assistant messages.
                    contents.push(serde_json::json!({
                        "role": "model",
                        "parts": [{"text": m.content}]
                    }));
                }
            }
        }

        // 2. Build generation config (max output tokens, optional structured output).
        let mut generation_config = serde_json::json!({
            "maxOutputTokens": max_tokens,
        });

        if let Some(schema) = response_schema {
            generation_config["responseMimeType"] = serde_json::json!("application/json");
            generation_config["responseSchema"] = strip_additional_properties(schema);
        }

        if reasoning_disabled {
            generation_config["thinkingConfig"] = serde_json::json!({"thinkingBudget": 0});
        }

        // 3. Build request body.
        let mut body = serde_json::json!({
            "contents": contents,
            "generationConfig": generation_config,
        });

        if !system_parts.is_empty() {
            body["systemInstruction"] = serde_json::json!({
                "parts": system_parts,
            });
        }

        // 4. POST to Gemini API.
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            model
        );

        let response = self
            .client
            .post(&url)
            .header("x-goog-api-key", &self.api_key)
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

        // 6. Extract text from candidates[0].content.parts[0].text.
        let content = response_json["candidates"][0]["content"]["parts"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();

        // 7. Map finish reason.
        let finish_reason_str = response_json["candidates"][0]["finishReason"]
            .as_str()
            .unwrap_or("STOP");

        let finish_reason = match finish_reason_str {
            "STOP" => FinishReason::Stop,
            "MAX_TOKENS" => FinishReason::Length,
            "SAFETY"
            | "RECITATION"
            | "BLOCKLIST"
            | "PROHIBITED_CONTENT"
            | "SPII"
            | "MALFORMED_FUNCTION_CALL" => FinishReason::ContentFilter,
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

fn strip_additional_properties(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut cleaned = serde_json::Map::new();
            for (k, v) in map {
                if k == "additionalProperties" {
                    continue;
                }
                cleaned.insert(k.clone(), strip_additional_properties(v));
            }
            Value::Object(cleaned)
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(strip_additional_properties).collect())
        }
        other => other.clone(),
    }
}

#[cfg(test)]
#[path = "../../../tests/engine_llm_google_provider_tests.rs"]
mod tests;
