// Document URL stored as collected.
// Validated via url::Url::parse but stores the original string unchanged — no normalization.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    Invalid(String),
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UrlError::Invalid(msg) => write!(f, "invalid URL: {}", msg),
        }
    }
}

impl Url {
    /// Validate via `url::Url::parse` but store the original input unchanged.
    /// `url::Url` normalizes (lowercases host, adds trailing slash); we avoid that
    /// to prevent silent state rewrites and broken duplicate-URL detection.
    pub fn parse(s: &str) -> Result<Self, UrlError> {
        ::url::Url::parse(s).map_err(|e| UrlError::Invalid(e.to_string()))?;
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Url {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Url {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[path = "../tests/domain_url_tests.rs"]
mod tests;
