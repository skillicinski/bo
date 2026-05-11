use async_openai::{
    config::OpenAIConfig,
    error::{ApiError, OpenAIError},
    types::chat::{
        ChatCompletionRequestMessage, ChatCompletionRequestSystemMessageArgs,
        ChatCompletionRequestUserMessageArgs, CreateChatCompletionRequestArgs,
        FinishReason as OaiFinishReason, ResponseFormat, ResponseFormatJsonSchema,
    },
    Client,
};
use async_trait::async_trait;
use serde_json::Value;
use std::time::Duration;

use crate::engine::llm::{FinishReason, LlmError, LlmProvider, LlmResponse, Message, Role};

pub struct OpenAiProvider {
    client: Client<OpenAIConfig>,
}

impl OpenAiProvider {
    pub fn new(api_key: &str) -> Self {
        let config = OpenAIConfig::new().with_api_key(api_key);
        let mut backoff_builder = backoff::ExponentialBackoffBuilder::new();
        backoff_builder.with_max_elapsed_time(Some(Duration::ZERO));
        Self {
            client: Client::build(reqwest::Client::new(), config, backoff_builder.build()),
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
            .map_err(map_openai_error)?;

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

fn map_openai_error(error: OpenAIError) -> LlmError {
    match error {
        OpenAIError::Reqwest(e) => LlmError::Network(e.to_string()),
        OpenAIError::ApiError(api_error) => map_api_error(api_error),
        OpenAIError::JSONDeserialize(e, body) => LlmError::Parse(format!("{}; body: {}", e, body)),
        OpenAIError::InvalidArgument(message) => LlmError::Parse(message),
        OpenAIError::FileSaveError(message) | OpenAIError::FileReadError(message) => {
            LlmError::Api(message)
        }
        OpenAIError::StreamError(error) => LlmError::Network(error.to_string()),
    }
}

fn map_api_error(error: ApiError) -> LlmError {
    let message = error.to_string();
    let code = error.code.as_deref().unwrap_or_default();
    let error_type = error.r#type.as_deref().unwrap_or_default();
    let lower_message = message.to_lowercase();

    if code.contains("rate_limit")
        || error_type.contains("rate_limit")
        || lower_message.contains("rate limit")
        || lower_message.contains("rate_limit")
    {
        return LlmError::RateLimited(message);
    }

    if error_type == "insufficient_quota" || code == "insufficient_quota" {
        return LlmError::Api(message);
    }

    if error.r#type.is_none() && error.code.is_none() && error.param.is_none() {
        return LlmError::Server(message);
    }

    LlmError::Api(message)
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
