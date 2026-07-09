// Validated model identifier.
//
// A `Model` is guaranteed to reference a supported model from the
// provider's model registry, or any non-empty model id for the custom
// provider. Context window size is available without a fallible lookup.

use super::{models, Provider};
use std::fmt;

/// A validated model identifier known to be in the provider's supported set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    id: String,
    context_tokens: usize,
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
    /// Parse and validate a model identifier against the provider's supported
    /// set. The custom and Google providers have no hard registry: any non-empty
    /// id is valid and gets the provider's default context window unless the
    /// known-model table has an entry for it.
    pub fn parse(id: &str, provider: Provider) -> Result<Self, UnsupportedModel> {
        let trimmed = id.trim();
        match provider {
            Provider::Custom | Provider::Google => {
                if trimmed.is_empty() {
                    return Err(UnsupportedModel {
                        id: String::new(),
                        provider,
                    });
                }
                let context_tokens = match provider {
                    Provider::Custom => models::CUSTOM_CONTEXT_TOKENS,
                    Provider::Google => models::find_model(provider, trimmed)
                        .map(|entry| entry.context_tokens)
                        .unwrap_or(models::GOOGLE_CONTEXT_TOKENS),
                    _ => unreachable!(),
                };
                Ok(Self {
                    id: trimmed.to_string(),
                    context_tokens,
                })
            }
            _ => {
                let info =
                    models::find_model(provider, trimmed).ok_or_else(|| UnsupportedModel {
                        id: trimmed.to_string(),
                        provider,
                    })?;
                Ok(Self {
                    id: info.id.to_string(),
                    context_tokens: info.context_tokens,
                })
            }
        }
    }

    /// The raw model identifier string.
    pub fn as_str(&self) -> &str {
        &self.id
    }

    /// Context window size in tokens. Infallible — validated at construction.
    pub fn context_tokens(&self) -> usize {
        self.context_tokens
    }
}

impl fmt::Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.id)
    }
}

#[cfg(test)]
#[path = "../../tests/engine_llm_model_tests.rs"]
mod tests;
