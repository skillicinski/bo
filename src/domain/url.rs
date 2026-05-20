// Url newtype — validated URL string at the domain boundary.
//
// Minimal validation: non-empty and contains "://". We store URLs as raw strings
// end-to-end (deduplication, frontmatter, etc.). Constructor opacity matters more
// than full RFC compliance.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Url(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlError {
    Empty,
    NoScheme,
}

impl fmt::Display for UrlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UrlError::Empty => write!(f, "URL cannot be empty"),
            UrlError::NoScheme => write!(f, "URL must contain a scheme separator (://)"),
        }
    }
}

impl Url {
    pub fn parse(s: &str) -> Result<Self, UrlError> {
        if s.is_empty() {
            return Err(UrlError::Empty);
        }
        if !s.contains("://") {
            return Err(UrlError::NoScheme);
        }
        Ok(Self(s.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for Url {
    fn as_ref(&self) -> &str {
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
