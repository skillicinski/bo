// Validated model identifier.
//
// A `Model` is guaranteed to reference a supported model from
// `OPENAI_SUPPORTED_MODELS`. Context window size is available without
// a fallible lookup.

use super::models::{self, ModelInfo, OPENAI_SUPPORTED_MODELS};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A validated model identifier known to be in `OPENAI_SUPPORTED_MODELS`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Model {
    info: &'static ModelInfo,
}

/// Error returned when a model identifier is not in the supported set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsupportedModel {
    pub id: String,
}

impl fmt::Display for UnsupportedModel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unsupported model: {}", self.id)
    }
}

impl std::error::Error for UnsupportedModel {}

impl Model {
    /// Parse and validate a model identifier against the supported set.
    pub fn parse(id: &str) -> Result<Self, UnsupportedModel> {
        let trimmed = id.trim();
        let info = OPENAI_SUPPORTED_MODELS
            .iter()
            .find(|entry| entry.id == trimmed)
            .ok_or_else(|| UnsupportedModel {
                id: trimmed.to_string(),
            })?;
        Ok(Self { info })
    }

    /// The default model (`gpt-4o`).
    pub fn default_model() -> Self {
        Self::parse(models::DEFAULT_MODEL).expect("default model must be in supported set")
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

impl PartialEq<&str> for Model {
    fn eq(&self, other: &&str) -> bool {
        self.info.id == *other
    }
}

impl Serialize for Model {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.info.id)
    }
}

impl<'de> Deserialize<'de> for Model {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(|e| serde::de::Error::custom(e.to_string()))
    }
}

#[cfg(test)]
#[path = "../../tests/engine_llm_model_tests.rs"]
mod tests;
