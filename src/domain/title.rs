// Document title.
// Validated newtype: non-empty after trimming.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TitleError {
    Empty,
}

impl fmt::Display for TitleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TitleError::Empty => write!(f, "title cannot be empty"),
        }
    }
}

impl Title {
    /// Parse and validate a title. Trims whitespace; rejects empty/whitespace-only.
    pub fn parse(s: &str) -> Result<Self, TitleError> {
        let trimmed = s.trim();
        if trimmed.is_empty() {
            return Err(TitleError::Empty);
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Title {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Title {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Title {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

// ponytail: pre-v0.0.6 tree files wrote title: "" for untitled leaves; fold to None on read.
// Remove once no pre-v0.0.6 trees remain.
pub(crate) fn deserialize_option_title<'de, D>(deserializer: D) -> Result<Option<Title>, D::Error>
where
    D: Deserializer<'de>,
{
    let opt: Option<String> = Option::deserialize(deserializer)?;
    match opt {
        None => Ok(None),
        Some(s) if s.trim().is_empty() => Ok(None),
        Some(s) => Title::parse(&s).map(Some).map_err(serde::de::Error::custom),
    }
}

#[cfg(test)]
#[path = "../tests/domain_title_tests.rs"]
mod tests;
