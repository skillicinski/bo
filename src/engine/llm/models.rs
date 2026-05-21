// Provider-aware model metadata.
//
// Each provider has its own supported model list. Functions dispatch on the
// Provider enum to return the correct list.

use super::Provider;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: &'static str,
    pub context_tokens: usize,
}

pub const DEFAULT_MODEL: &str = "gpt-4.1-mini";

pub const OPENAI_MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "gpt-4o",
        context_tokens: 128_000,
    },
    ModelInfo {
        id: "gpt-4o-mini",
        context_tokens: 128_000,
    },
    ModelInfo {
        id: "gpt-4.1",
        context_tokens: 1_000_000,
    },
    ModelInfo {
        id: "gpt-4.1-mini",
        context_tokens: 1_000_000,
    },
    ModelInfo {
        id: "gpt-4.1-nano",
        context_tokens: 1_000_000,
    },
];

pub const DEEPSEEK_MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "deepseek-v4-flash",
        context_tokens: 1_000_000,
    },
    ModelInfo {
        id: "deepseek-v4-pro",
        context_tokens: 1_000_000,
    },
];

pub fn models_for(provider: Provider) -> &'static [ModelInfo] {
    match provider {
        Provider::OpenAI => OPENAI_MODELS,
        Provider::Deepseek => DEEPSEEK_MODELS,
    }
}

pub fn is_supported_model(provider: Provider, model_id: &str) -> bool {
    let model_id = model_id.trim();
    models_for(provider)
        .iter()
        .any(|entry| entry.id == model_id)
}

pub fn context_window_tokens(provider: Provider, model_id: &str) -> Option<usize> {
    let model_id = model_id.trim();
    models_for(provider)
        .iter()
        .find(|entry| entry.id == model_id)
        .map(|entry| entry.context_tokens)
}
