use async_trait::async_trait;
use serde_json::Value;

use super::{map_http_error, map_reqwest_error};
use crate::engine::llm::{
    sanitize_provider_error_message, FinishReason, LlmError, LlmProvider, LlmResponse, Message,
    NormalizedSchema, Role,
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
    /// Gemini's `responseSchema` accepts a subset of OpenAPI 3.0 Schema.
    /// `additionalProperties: false` is not part of that subset — strip it.
    /// Reject anything else (e.g. `additionalProperties: { … }`) as unsupported.
    fn map_response_schema(&self, schema: &Value) -> Result<NormalizedSchema, LlmError> {
        to_gemini_schema(schema).map(NormalizedSchema)
    }

    async fn complete(
        &self,
        messages: &[Message],
        model: &str,
        max_tokens: u32,
        response_schema: Option<&NormalizedSchema>,
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
            }
        }

        // 2. Build generation config (max output tokens, optional structured output).
        let mut generation_config = serde_json::json!({
            "maxOutputTokens": max_tokens,
        });

        if let Some(schema) = response_schema {
            generation_config["responseMimeType"] = serde_json::json!("application/json");
            generation_config["responseSchema"] = schema.0.clone();
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

/// Recursively strip `additionalProperties: false` from a JSON Schema value.
///
/// Gemini's `responseSchema` uses a subset of OpenAPI 3.0 Schema that does
/// not accept `additionalProperties`. A value of `false` ("no extra props")
/// is safe to strip — the schema's meaning is unchanged in Gemini's dialect.
///
/// Any other value (e.g. `additionalProperties: { "type": "string" }`)
/// signals a schema constraint Gemini cannot express — reject it explicitly.
fn to_gemini_schema(value: &Value) -> Result<Value, LlmError> {
    match value {
        Value::Object(map) => {
            let mut cleaned = serde_json::Map::new();
            for (k, v) in map {
                if k == "additionalProperties" {
                    match v {
                        Value::Bool(false) => continue, // safe to strip
                        other => {
                            return Err(LlmError::Api(format!(
                                "Gemini does not support additionalProperties: {other}"
                            )));
                        }
                    }
                }
                cleaned.insert(k.clone(), to_gemini_schema(v)?);
            }
            Ok(Value::Object(cleaned))
        }
        Value::Array(items) => {
            let mapped: Result<Vec<_>, _> = items.iter().map(to_gemini_schema).collect();
            Ok(Value::Array(mapped?))
        }
        other => Ok(other.clone()),
    }
}

#[cfg(test)]
#[path = "../../../tests/engine_llm_google_provider_tests.rs"]
mod tests;
