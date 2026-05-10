use super::*;
use std::fs;
use tempfile::TempDir;

#[test]
fn basic_ascii_title() {
    assert_eq!(
        slugify("Rust Ownership Explained", ""),
        "rust-ownership-explained"
    );
}

#[test]
fn special_characters() {
    assert_eq!(slugify("Hello, World! (2024)", ""), "hello-world-2024");
}

#[test]
fn collapses_hyphens() {
    assert_eq!(slugify("foo---bar   baz", ""), "foo-bar-baz");
}

#[test]
fn strips_leading_trailing() {
    assert_eq!(slugify("  --hello-- ", ""), "hello");
}

#[test]
fn truncates_at_80_chars() {
    let long_title = "this-is-a-very-long-title-that-exceeds-eighty-characters-and-should-be-truncated-at-a-hyphen-boundary";
    let slug = slugify(long_title, "");
    assert!(slug.len() <= 80, "slug too long: {} chars", slug.len());
    assert!(!slug.ends_with('-'), "slug ends with hyphen");
}

#[test]
fn empty_title_falls_back_to_url() {
    let slug = slugify("", "https://example.com/some/great-article");
    assert_eq!(slug, "example-com-some-great-article");
}

#[test]
fn non_ascii_title_falls_back_to_url() {
    let slug = slugify("日本語のタイトル", "https://example.com/jp/article");
    assert_eq!(slug, "example-com-jp-article");
}

#[test]
fn collision_adds_hash() {
    let dir = TempDir::new().unwrap();
    // Create an existing file to force collision
    fs::write(dir.path().join("introduction.md"), "existing").unwrap();

    let resolved = resolve_slug("introduction", "https://example.com/intro1", dir.path());
    assert_ne!(resolved, "introduction");
    assert!(resolved.starts_with("introduction-"));
    assert_eq!(resolved.len(), "introduction-".len() + 12); // 6 bytes = 12 hex chars
}

#[test]
fn no_collision_no_hash() {
    let dir = TempDir::new().unwrap();
    let resolved = resolve_slug("introduction", "https://example.com/intro1", dir.path());
    assert_eq!(resolved, "introduction");
}

#[test]
fn different_urls_get_different_hashes() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("introduction.md"), "existing").unwrap();

    let r1 = resolve_slug("introduction", "https://example.com/intro1", dir.path());
    let r2 = resolve_slug("introduction", "https://example.com/intro2", dir.path());
    assert_ne!(r1, r2);
}

#[test]
fn url_only_hash_fallback() {
    // Totally degenerate case: no title, URL is just a domain
    let slug = slugify("", "https://例え.jp/");
    assert!(!slug.is_empty(), "slug should not be empty");
}
