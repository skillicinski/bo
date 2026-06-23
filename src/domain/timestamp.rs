// Timestamp newtype — wraps chrono::DateTime<Utc> with validated construction.
//
// Serializes as RFC 3339 string (milliseconds, Z suffix). Implements Ord for
// typed comparisons that replace lexicographic string tricks.

use chrono::{DateTime, SecondsFormat, Timelike, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Timestamp(DateTime<Utc>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimestampError {
    InvalidFormat(String),
}

impl fmt::Display for TimestampError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TimestampError::InvalidFormat(msg) => write!(f, "invalid timestamp: {}", msg),
        }
    }
}

impl Timestamp {
    pub fn parse(s: &str) -> Result<Self, TimestampError> {
        DateTime::parse_from_rfc3339(s)
            .map(|dt| Self(dt.with_timezone(&Utc)))
            .map_err(|e| TimestampError::InvalidFormat(e.to_string()))
    }

    pub fn now() -> Self {
        let now = Utc::now();
        let millis = now.nanosecond() / 1_000_000 * 1_000_000;
        Self(now.with_nanosecond(millis).expect("nanos in range"))
    }

    pub fn to_rfc3339_millis(&self) -> String {
        self.0.to_rfc3339_opts(SecondsFormat::Millis, true)
    }
}

impl PartialOrd for Timestamp {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Timestamp {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.cmp(&other.0)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_rfc3339_millis())
    }
}

impl Serialize for Timestamp {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_rfc3339_millis())
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
#[path = "../tests/domain_timestamp_tests.rs"]
mod tests;
