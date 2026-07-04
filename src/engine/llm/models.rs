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

// Z.ai GLM Coding Plan (subscription) models, served from the coding endpoint
// (https://api.z.ai/api/coding/paas/v4). Context windows per Z.ai's coding-tool
// config guidance: glm-5.2 = 1M, others = 200k. Z.ai recommends glm-4.7 for
// routine tasks (1x quota); glm-5.2 / glm-5-turbo consume 2–3x quota.
//
// The plan also exposes glm-5v-turbo (vision), excluded here since bo is
// text-only. Pay-per-token PaaS-only models (glm-4.6, glm-4.5-x/airx,
// glm-4-32b, glm-4.7-flash/flashx) aren't on the coding plan and aren't listed;
// add the standard PaaS endpoint as a separate provider if needed.
pub const ZAI_MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "glm-4.7",
        context_tokens: 200_000,
    },
    ModelInfo {
        id: "glm-4.5-air",
        context_tokens: 200_000,
    },
    ModelInfo {
        id: "glm-5.1",
        context_tokens: 200_000,
    },
    ModelInfo {
        id: "glm-5-turbo",
        context_tokens: 200_000,
    },
    ModelInfo {
        id: "glm-5.2",
        context_tokens: 1_000_000,
    },
];

pub const GOOGLE_MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "gemini-2.5-flash-lite",
        context_tokens: 1_048_576,
    },
    ModelInfo {
        id: "gemini-2.5-flash",
        context_tokens: 1_048_576,
    },
    ModelInfo {
        id: "gemini-2.5-pro",
        context_tokens: 1_048_576,
    },
];

pub fn models_for(provider: Provider) -> &'static [ModelInfo] {
    match provider {
        Provider::OpenAI => OPENAI_MODELS,
        Provider::Deepseek => DEEPSEEK_MODELS,
        Provider::Google => GOOGLE_MODELS,
        Provider::Zai => ZAI_MODELS,
    }
}

pub(crate) fn find_model(provider: Provider, model_id: &str) -> Option<&'static ModelInfo> {
    let model_id = model_id.trim();
    models_for(provider)
        .iter()
        .find(|entry| entry.id == model_id)
}

pub fn is_supported_model(provider: Provider, model_id: &str) -> bool {
    find_model(provider, model_id).is_some()
}

pub fn context_window_tokens(provider: Provider, model_id: &str) -> Option<usize> {
    find_model(provider, model_id).map(|entry| entry.context_tokens)
}
