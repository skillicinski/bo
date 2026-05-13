// Release-supported model metadata.
//
// The current provider surface is OpenAI-only. These model IDs and context
// windows are OpenAI model capabilities, not a provider-agnostic namespace.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelInfo {
    pub id: &'static str,
    pub context_tokens: usize,
}

pub const DEFAULT_MODEL: &str = "gpt-4o";

pub const OPENAI_SUPPORTED_MODELS: &[ModelInfo] = &[
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

pub fn supported_model_ids() -> impl Iterator<Item = &'static str> {
    OPENAI_SUPPORTED_MODELS.iter().map(|entry| entry.id)
}

pub fn is_supported_model(model: &str) -> bool {
    model_info(model).is_some()
}

pub fn context_window_tokens(model: &str) -> Option<usize> {
    model_info(model).map(|entry| entry.context_tokens)
}

fn model_info(model: &str) -> Option<&'static ModelInfo> {
    let model = model.trim();
    OPENAI_SUPPORTED_MODELS
        .iter()
        .find(|entry| entry.id == model)
}
