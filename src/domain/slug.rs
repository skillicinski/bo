// Slug newtype — validated kebab-case identifier for domain entities.
//
// Validated: ASCII alphanumeric + hyphens, no leading/trailing/consecutive hyphens,
// length ≤ 80.
//
// Two constructors:
//   Slug::parse(s)          — validates an existing string (from disk/state)
//   Slug::generate(title, url) — infallible generation from title+url

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;

// ── types ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Slug(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlugError {
    Empty,
    TooLong(usize),
    InvalidChar(char),
    LeadingHyphen,
    TrailingHyphen,
    ConsecutiveHyphens,
}

impl fmt::Display for SlugError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SlugError::Empty => write!(f, "slug cannot be empty"),
            SlugError::TooLong(len) => write!(f, "slug too long: {} chars (max 80)", len),
            SlugError::InvalidChar(c) => write!(f, "slug contains invalid char: {:?}", c),
            SlugError::LeadingHyphen => write!(f, "slug cannot start with a hyphen"),
            SlugError::TrailingHyphen => write!(f, "slug cannot end with a hyphen"),
            SlugError::ConsecutiveHyphens => write!(f, "slug cannot contain consecutive hyphens"),
        }
    }
}

impl Slug {
    /// Validate an existing string as a slug (from disk/state).
    pub fn parse(s: &str) -> Result<Self, SlugError> {
        if s.is_empty() {
            return Err(SlugError::Empty);
        }
        if s.len() > 80 {
            return Err(SlugError::TooLong(s.len()));
        }
        if s.starts_with('-') {
            return Err(SlugError::LeadingHyphen);
        }
        if s.ends_with('-') {
            return Err(SlugError::TrailingHyphen);
        }
        let mut prev_hyphen = false;
        for c in s.chars() {
            if c == '-' {
                if prev_hyphen {
                    return Err(SlugError::ConsecutiveHyphens);
                }
                prev_hyphen = true;
            } else if c.is_ascii_alphanumeric() {
                prev_hyphen = false;
            } else {
                return Err(SlugError::InvalidChar(c));
            }
        }
        Ok(Self(s.to_string()))
    }

    /// Generate a slug from title + url. Infallible by construction.
    pub fn generate(title: &str, url: &str) -> Self {
        let raw = slugify_raw(title);
        let s = if raw.is_empty() {
            slugify_from_url(url)
        } else {
            raw
        };
        // The generation pipeline guarantees the output is valid.
        Self(s)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for Slug {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Slug {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

// ── public helpers ────────────────────────────────────────────────────────────

/// Allocate a unique slug, appending a hash suffix on collision.
///
/// `used` holds already-claimed filename stems. The caller seeds it with
/// existing on-disk stems (and, in batch contexts, stems claimed earlier
/// in the batch). If `slug` is not in `used`, it is inserted and returned
/// as-is. Otherwise a URL-hash suffix is appended and the disambiguated
/// stem is also inserted into `used`.
pub fn allocate_slug(slug: &Slug, url: &str, used: &mut std::collections::HashSet<String>) -> Slug {
    if used.insert(slug.as_str().to_string()) {
        return slug.clone();
    }
    // Collision: append hash suffix. Truncate base slug so the total
    // stays ≤ 80 chars (12 hex + 1 hyphen = 13 chars reserved).
    let hash = url_hash(url);
    let base = truncate_at_boundary(slug.as_str(), 80 - 1 - 12);
    let resolved = format!("{}-{}", base, hash);
    used.insert(resolved.clone());
    Slug(resolved)
}

// ── internals ─────────────────────────────────────────────────────────────────

fn slugify_raw(input: &str) -> String {
    let lower = input.to_lowercase();
    let mut slug = String::with_capacity(lower.len());

    for c in lower.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if c == '-' || c == ' ' || c == '_' || c == '.' || c == '/' {
            slug.push('-');
        }
        // Drop non-ASCII and other special chars
    }

    // Collapse consecutive hyphens
    let mut collapsed = String::with_capacity(slug.len());
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                collapsed.push('-');
            }
            prev_hyphen = true;
        } else {
            collapsed.push(c);
            prev_hyphen = false;
        }
    }

    // Strip leading/trailing hyphens
    let trimmed = collapsed.trim_matches('-').to_string();

    // Truncate to 80 chars at a hyphen boundary
    truncate_at_boundary(&trimmed, 80)
}

fn slugify_from_url(url: &str) -> String {
    let slug = url::Url::parse(url).ok().and_then(|parsed| {
        let mut source = parsed.host_str().unwrap_or_default().to_string();
        source.push_str(parsed.path());
        let slug = slugify_raw(source.trim_matches('/'));
        (!slug.is_empty()).then_some(slug)
    });

    slug.unwrap_or_else(|| url_hash(url))
}

fn truncate_at_boundary(s: &str, max: usize) -> String {
    debug_assert!(s.is_ascii(), "slug truncation requires ASCII input");
    if s.len() <= max {
        return s.to_string();
    }
    // Find the last hyphen before max
    let truncated = &s[..max];
    if let Some(pos) = truncated.rfind('-') {
        truncated[..pos].to_string()
    } else {
        truncated.to_string()
    }
}

fn url_hash(url: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(url.as_bytes());
    let result = hasher.finalize();
    result[..6]
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

#[cfg(test)]
#[path = "../tests/domain_slug_tests.rs"]
mod tests;
