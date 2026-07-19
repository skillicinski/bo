use async_trait::async_trait;
use serde_json::Value;

use super::{map_http_error, map_reqwest_error};
use crate::engine::llm::{
    sanitize_provider_error_message, AgentMessage, AgentResponse, FinishReason, LlmError,
    LlmProvider, LlmResponse, Message, ProviderSchema, Role, ToolCall, ToolSchema, Usage,
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

    /// Any OpenAI-compatible endpoint. `base_url` is the prefix before
    /// `/chat/completions`, e.g. `https://api.example.com/v1`.
    pub fn custom(api_key: &str, base_url: &str) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.to_string(),
            base_url: format!("{}/chat/completions", base_url.trim_end_matches('/')),
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
        response_schema: Option<&ProviderSchema>,
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

    async fn complete_with_tools(
        &self,
        messages: &[AgentMessage],
        model: &str,
        max_tokens: u32,
        tools: &[ToolSchema],
        reasoning_disabled: bool,
    ) -> Result<AgentResponse, LlmError> {
        let api_messages: Vec<Value> = messages.iter().map(agent_message_to_json).collect();

        let mut body = serde_json::json!({
            "model": model,
            "messages": api_messages,
            "max_tokens": max_tokens,
        });

        if !tools.is_empty() {
            body["tools"] = serde_json::Value::Array(
                tools
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "type": "function",
                            "function": {
                                "name": t.name,
                                "description": t.description,
                                "parameters": t.parameters,
                            }
                        })
                    })
                    .collect(),
            );
            body["tool_choice"] = serde_json::json!("auto");
        }

        if reasoning_disabled {
            body["thinking"] = serde_json::json!({"type": "disabled"});
        }

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

        let response_json: Value = serde_json::from_str(&response_text).map_err(|e| {
            let sanitized = sanitize_provider_error_message(&response_text);
            LlmError::Parse(format!("{}; body: {}", e, sanitized))
        })?;

        let message = &response_json["choices"][0]["message"];
        let content = message["content"].as_str().map(|s| s.to_string());
        // DeepSeek returns reasoning_content in thinking mode. Replay it on the
        // next turn or the provider rejects the request with HTTP 400.
        let reasoning_content = message["reasoning_content"].as_str().map(|s| s.to_string());

        let tool_calls: Vec<ToolCall> = message["tool_calls"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|tc| {
                        let id = tc["id"].as_str()?.to_string();
                        let name = tc["function"]["name"].as_str()?.to_string();
                        // arguments is a JSON string; keep it raw for the tool boundary.
                        let arguments = tc["function"]["arguments"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        Some(ToolCall {
                            id,
                            name,
                            arguments,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let finish_reason_str = response_json["choices"][0]["finish_reason"]
            .as_str()
            .unwrap_or("stop");
        // DeepSeek/OpenAI signal tool calls via finish_reason: "tool_calls".
        let finish_reason = match finish_reason_str {
            "stop" => FinishReason::Stop,
            "length" => FinishReason::Length,
            "content_filter" => FinishReason::ContentFilter,
            other => FinishReason::Other(other.to_string()),
        };

        let usage = response_json.get("usage").map(|u| Usage {
            prompt_tokens: u["prompt_tokens"].as_u64().unwrap_or(0),
            completion_tokens: u["completion_tokens"].as_u64().unwrap_or(0),
            total_tokens: u["total_tokens"].as_u64().unwrap_or(0),
        });

        Ok(AgentResponse {
            content,
            reasoning_content,
            tool_calls,
            finish_reason,
            usage,
        })
    }
}

/// Serialize an agent transcript message into the OpenAI chat format.
///
/// Assistant messages replay `reasoning_content` (DeepSeek requirement) and
/// carry `tool_calls`; tool results use role `tool` with the matching
/// `tool_call_id`. System messages use role `system` (not `developer`).
fn agent_message_to_json(message: &AgentMessage) -> Value {
    match message {
        AgentMessage::System(content) => serde_json::json!({
            "role": "system",
            "content": content,
        }),
        AgentMessage::User(content) => serde_json::json!({
            "role": "user",
            "content": content,
        }),
        AgentMessage::Tool(result) => serde_json::json!({
            "role": "tool",
            "tool_call_id": result.tool_call_id,
            "content": result.content,
        }),
        AgentMessage::Assistant {
            content,
            reasoning_content,
            tool_calls,
        } => {
            let mut map = serde_json::Map::new();
            map.insert("role".to_string(), Value::String("assistant".to_string()));
            map.insert(
                "content".to_string(),
                content.clone().map(Value::String).unwrap_or(Value::Null),
            );
            if let Some(rc) = reasoning_content {
                map.insert("reasoning_content".to_string(), Value::String(rc.clone()));
            }
            if !tool_calls.is_empty() {
                let calls: Vec<Value> = tool_calls
                    .iter()
                    .map(|tc| {
                        serde_json::json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.arguments,
                            }
                        })
                    })
                    .collect();
                map.insert("tool_calls".to_string(), Value::Array(calls));
            }
            Value::Object(map)
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/engine_llm_openai_compat_provider_tests.rs"]
mod tests;
