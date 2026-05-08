use async_openai::{
    config::OpenAIConfig,
    types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
        FinishReason as OaiFinishReason, ResponseFormat, ResponseFormatJsonSchema,
    },
    Client,
};
use async_trait::async_trait;
use serde_json::Value;

use crate::engine::llm::{FinishReason, LlmError, LlmProvider, LlmResponse, Message, Role};

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
    async fn complete(
        &self,
        messages: &[Message],
        model: &str,
        max_tokens: u32,
        response_schema: Option<&Value>,
    ) -> Result<LlmResponse, LlmError> {
        let api_messages: Vec<ChatCompletionRequestMessage> = messages
            .iter()
            .map(to_api_message)
            .collect::<Result<_, _>>()?;

        let mut builder = CreateChatCompletionRequestArgs::default();
        builder
            .model(model)
            .max_completion_tokens(max_tokens)
            .messages(api_messages);

        if let Some(schema) = response_schema {
            builder.response_format(ResponseFormat::JsonSchema {
                json_schema: ResponseFormatJsonSchema {
                    name: "response".to_string(),
                    description: None,
                    schema: Some(schema.clone()),
                    strict: Some(true),
                },
            });
        }

        let request = builder
            .build()
            .map_err(|e| LlmError::Parse(e.to_string()))?;

        let response = self
            .client
            .chat()
            .create(request)
            .await
            .map_err(|e| LlmError::Api(e.to_string()))?;

        let choice = response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| LlmError::Parse("no choices in response".into()))?;

        let finish_reason = match choice.finish_reason {
            Some(OaiFinishReason::Stop) => FinishReason::Stop,
            Some(OaiFinishReason::Length) => FinishReason::Length,
            Some(OaiFinishReason::ContentFilter) => FinishReason::ContentFilter,
            other => FinishReason::Other(format!("{:?}", other)),
        };

        let content = choice.message.content.unwrap_or_default();

        Ok(LlmResponse {
            content,
            finish_reason,
        })
    }
}

fn to_api_message(m: &Message) -> Result<ChatCompletionRequestMessage, LlmError> {
    match m.role {
        Role::System => Ok(ChatCompletionRequestMessage::System(
            ChatCompletionRequestSystemMessageArgs::default()
                .content(m.content.clone())
                .build()
                .map_err(|e| LlmError::Parse(e.to_string()))?,
        )),
        Role::User => Ok(ChatCompletionRequestMessage::User(
            ChatCompletionRequestUserMessageArgs::default()
                .content(m.content.clone())
                .build()
                .map_err(|e| LlmError::Parse(e.to_string()))?,
        )),
        Role::Assistant => {
            unreachable!("Assistant messages are not used in the compile pipeline")
        }
    }
}
