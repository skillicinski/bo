pub mod deepseek;
pub mod google;
pub mod openai;

use crate::engine::llm::{sanitize_provider_error_message, LlmError};

pub use deepseek::DeepSeekProvider;
pub use google::GoogleProvider;
pub use openai::OpenAiProvider;

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
