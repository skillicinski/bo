use async_trait::async_trait;
use serde_json::Value;

use async_openai::{
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionMessageToolCalls, ChatCompletionRequestAssistantMessageArgs,
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestToolMessage, ChatCompletionRequestUserMessageArgs, ChatCompletionTool,
        ChatCompletionTools, CreateChatCompletionRequestArgs, FunctionObjectArgs,
    },
    Client,
};

use crate::engine::agent::{AgentError, Completion, LlmProvider, Message, Role, Tool, ToolCall};

pub struct OpenAiProvider {
    client: Client<OpenAIConfig>,
}

impl OpenAiProvider {
    pub fn new(api_key: &str) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        Self {
            client: Client::with_config(config),
        }
    }
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "OpenAI"
    }

    async fn complete(
        &self,
        messages: &[Message],
        tools: &[&dyn Tool],
        model: &str,
    ) -> Result<Completion, AgentError> {
        // Build tool specs — wrap each in the ChatCompletionTools enum
        let tool_specs: Vec<ChatCompletionTools> = tools
            .iter()
            .map(|t| {
                ChatCompletionTools::Function(ChatCompletionTool {
                    function: FunctionObjectArgs::default()
                        .name(t.name())
                        .description(t.description())
                        .parameters(t.parameters_schema())
                        .build()
                        .expect("tool spec build failed"),
                })
            })
            .collect();

        // Convert messages
        let api_messages: Vec<ChatCompletionRequestMessage> = messages
            .iter()
            .map(to_api_message)
            .collect::<Result<_, _>>()?;

        let request = CreateChatCompletionRequestArgs::default()
            .model(model)
            .max_completion_tokens(4096u32)
            .messages(api_messages)
            .tools(tool_specs)
            .build()
            .map_err(|e| AgentError::Parse(e.to_string()))?;

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| AgentError::Api(e.to_string()))?;

        let response_message = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AgentError::Parse("no choices in response".into()))?
            .message;

        let content = response_message.content.clone();
        let raw_tool_calls = response_message
            .tool_calls
            .as_ref()
            .map(|tcs| serde_json::to_value(tcs).unwrap_or(Value::Null));

        let mut tool_calls: Vec<ToolCall> = Vec::new();
        if let Some(tcs) = response_message.tool_calls {
            for tc_enum in tcs {
                if let ChatCompletionMessageToolCalls::Function(tc) = tc_enum {
                    let args: Value = tc
                        .function
                        .arguments
                        .parse()
                        .unwrap_or(Value::Object(serde_json::Map::new()));
                    tool_calls.push(ToolCall {
                        id: tc.id,
                        name: tc.function.name,
                        args,
                    });
                }
            }
        }

        Ok(Completion {
            content,
            tool_calls,
            tool_calls_raw: raw_tool_calls,
        })
    }
}

fn to_api_message(m: &Message) -> Result<ChatCompletionRequestMessage, AgentError> {
    match m.role {
        Role::System => Ok(ChatCompletionRequestMessage::System(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(m.content.clone())
                .build()
                .map_err(|e| AgentError::Parse(e.to_string()))?,
        )),
        Role::User => Ok(ChatCompletionRequestMessage::User(
            ChatCompletionRequestUserMessageArgs::default()
                .content(m.content.clone())
                .build()
                .map_err(|e| AgentError::Parse(e.to_string()))?,
        )),
        Role::Assistant => {
            let mut builder = ChatCompletionRequestAssistantMessageArgs::default();
            if let Some(ref raw) = m.tool_calls_raw {
                // Deserialise raw tool_calls JSON back into the typed list
                let typed_tcs: Vec<ChatCompletionMessageToolCalls> =
                    serde_json::from_value(raw.clone())
                        .map_err(|e| AgentError::Parse(e.to_string()))?;
                builder.tool_calls(typed_tcs);
            }
            if !m.content.is_empty() {
                builder.content(m.content.clone());
            }
            Ok(ChatCompletionRequestMessage::Assistant(
                builder
                    .build()
                    .map_err(|e| AgentError::Parse(e.to_string()))?,
            ))
        }
        Role::Tool => {
            let id = m
                .tool_call_id
                .clone()
                .ok_or_else(|| AgentError::Parse("tool message missing tool_call_id".into()))?;
            Ok(ChatCompletionRequestMessage::Tool(
                ChatCompletionRequestToolMessage {
                    content: m.content.clone().into(),
                    tool_call_id: id,
                },
            ))
        }
    }
}
