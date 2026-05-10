// Slug generation and collision resolution

use sha2::{Digest, Sha256};
use std::path::Path;

/// Generate a kebab-case slug from a title string.
/// Falls back to extracting a slug from the URL path if the title is empty/non-ASCII.
pub fn slugify(title: &str, url: &str) -> String {
    let slug = slugify_raw(title);
    if slug.is_empty() {
        slugify_from_url(url)
    } else {
        slug
    }
}

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

/// Resolve a slug to a unique filename, appending a hash suffix on collision.
pub fn resolve_slug(slug: &str, url: &str, output_dir: &Path) -> String {
    let candidate = format!("{}.md", slug);
    if !output_dir.join(&candidate).exists() {
        return slug.to_string();
    }
    // Collision: append hash suffix
    let hash = url_hash(url);
    format!("{}-{}", slug, hash)
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
