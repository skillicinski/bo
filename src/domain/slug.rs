// Slug newtype — validated kebab-case identifier for domain entities.
//
// Validated: ASCII alphanumeric + hyphens, no leading/trailing/consecutive hyphens,
// length ≤ 80.
//
// Two constructors:
//   Slug::parse(s)          — validates an existing string (from disk/manifest)
//   Slug::generate(title, url) — infallible generation from title+url

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use std::path::Path;

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
    /// Validate an existing string as a slug (from disk/manifest).
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

impl AsRef<str> for Slug {
    fn as_ref(&self) -> &str {
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

/// Resolve a slug to a unique one, appending a hash suffix on collision.
pub fn resolve_slug(slug: &Slug, url: &str, output_dir: &Path) -> Slug {
    let candidate = format!("{}.md", slug.as_str());
    if !output_dir.join(&candidate).exists() {
        return slug.clone();
    }
    // Collision: append hash suffix
    let hash = url_hash(url);
    let resolved = format!("{}-{}", slug.as_str(), hash);
    // The resolved slug is valid by construction (original slug + hyphen + hex chars).
    Slug(resolved)
}

// ── backward compat: free function preserved for callers that don't need the struct ──

/// Generate a kebab-case slug string from a title string.
/// Falls back to extracting a slug from the URL path if the title is empty/non-ASCII.
pub fn slugify(title: &str, url: &str) -> String {
    Slug::generate(title, url).0
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
    // Extract path from URL, strip extension, slugify
    let path = url
        .split("://")
        .nth(1)
        .unwrap_or(url)
        .split('?')
        .next()
        .unwrap_or("")
        .split('#')
        .next()
        .unwrap_or("")
        .trim_matches('/');

    let slug = slugify_raw(path);
    if slug.is_empty() {
        // Last resort: hash of the URL
        url_hash(url)
    } else {
        slug
    }
}

fn truncate_at_boundary(s: &str, max: usize) -> String {
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
    hex::encode(&result[..6]) // 6 bytes = 12 hex chars
}

// Inline hex encoding to avoid adding a dep
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{:02x}", b)).collect()
    }
}

#[cfg(test)]
#[path = "../tests/domain_slug_tests.rs"]
mod tests;
