// Validated model identifier.
//
// A `Model` is guaranteed to reference a supported model from the
// provider's model registry. Context window size is available without
// a fallible lookup.

use super::models::ModelInfo;
use super::{models, Provider};
use std::fmt;

/// A validated model identifier known to be in the provider's supported set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    info: &'static ModelInfo,
}

/// Error returned when a model identifier is not in the provider's supported set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedModel {
    pub id: String,
    pub provider: Provider,
}

impl fmt::Display for UnsupportedModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unsupported model '{}' for provider '{}'",
            self.id, self.provider
        )
    }
}

impl std::error::Error for UnsupportedModel {}

impl Model {
    /// Parse and validate a model identifier against the provider's supported set.
    pub fn parse(id: &str, provider: Provider) -> Result<Self, UnsupportedModel> {
        let trimmed = id.trim();
        let info = models::find_model(provider, trimmed).ok_or_else(|| UnsupportedModel {
            id: trimmed.to_string(),
            provider,
        })?;
        Ok(Self { info })
    }

    /// The raw model identifier string.
    pub fn as_str(&self) -> &str {
        self.info.id
    }

    /// Context window size in tokens. Infallible — validated at construction.
    pub fn context_tokens(&self) -> usize {
        self.info.context_tokens
    }
}

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.info.id)
    }
}

#[cfg(test)]
#[path = "../../tests/engine_llm_model_tests.rs"]
mod tests;
