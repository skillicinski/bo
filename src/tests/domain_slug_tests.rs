use super::*;
use std::collections::HashSet;

// ── Slug::generate (was slugify) ─────────────────────────────────────────────

#[test]
fn basic_ascii_title() {
    assert_eq!(
        Slug::generate("Rust Ownership Explained", "").to_string(),
        "rust-ownership-explained"
    );
}

#[test]
fn special_characters() {
    assert_eq!(
        Slug::generate("Hello, World! (2024)", "").to_string(),
        "hello-world-2024"
    );
}

#[test]
fn collapses_hyphens() {
    assert_eq!(
        Slug::generate("foo---bar   baz", "").to_string(),
        "foo-bar-baz"
    );
}

#[test]
fn strips_leading_trailing() {
    assert_eq!(Slug::generate("  --hello-- ", "").to_string(), "hello");
}

#[test]
fn truncates_at_80_chars() {
    let long_title = "this-is-a-very-long-title-that-exceeds-eighty-characters-and-should-be-truncated-at-a-hyphen-boundary";
    let slug = Slug::generate(long_title, "").to_string();
    assert!(slug.len() <= 80, "slug too long: {} chars", slug.len());
    assert!(!slug.ends_with('-'), "slug ends with hyphen");
}

#[test]
fn empty_title_falls_back_to_url() {
    let slug = Slug::generate("", "https://example.com/some/great-article").to_string();
    assert_eq!(slug, "example-com-some-great-article");
}

#[test]
fn non_ascii_title_falls_back_to_url() {
    let slug = Slug::generate("日本語のタイトル", "https://example.com/jp/article").to_string();
    assert_eq!(slug, "example-com-jp-article");
}

#[test]
fn collision_adds_hash() {
    let mut used = HashSet::from(["introduction".to_string()]);

    let base = Slug::parse("introduction").unwrap();
    let resolved = allocate_slug(&base, "https://example.com/intro1", &mut used);
    assert_ne!(resolved.as_str(), "introduction");
    assert!(resolved.as_str().starts_with("introduction-"));
    assert_eq!(resolved.as_str().len(), "introduction-".len() + 12); // 6 bytes = 12 hex chars
}

#[test]
fn no_collision_no_hash() {
    let mut used = HashSet::new();
    let base = Slug::parse("introduction").unwrap();
    let resolved = allocate_slug(&base, "https://example.com/intro1", &mut used);
    assert_eq!(resolved.as_str(), "introduction");
}

#[test]
fn different_urls_get_different_hashes() {
    let mut used = HashSet::from(["introduction".to_string()]);

    let base = Slug::parse("introduction").unwrap();
    let r1 = allocate_slug(&base, "https://example.com/intro1", &mut used);
    let r2 = allocate_slug(&base, "https://example.com/intro2", &mut used);
    assert_ne!(r1, r2);
}

#[test]
fn url_only_hash_fallback() {
    // Totally degenerate case: no title, URL is just a domain
    let slug = Slug::generate("", "https://例え.jp/").to_string();
    assert!(!slug.is_empty(), "slug should not be empty");
}

// ── Slug::parse ──────────────────────────────────────────────────────────────

#[test]
fn parse_valid_slug() {
    let s = Slug::parse("rust-ownership-explained").unwrap();
    assert_eq!(s.as_str(), "rust-ownership-explained");
}

#[test]
fn parse_single_word() {
    let s = Slug::parse("hello").unwrap();
    assert_eq!(s.as_str(), "hello");
}

#[test]
fn parse_with_numbers() {
    let s = Slug::parse("item-42-foo").unwrap();
    assert_eq!(s.as_str(), "item-42-foo");
}

#[test]
fn parse_rejects_empty() {
    assert!(matches!(Slug::parse(""), Err(SlugError::Empty)));
}

#[test]
fn parse_rejects_too_long() {
    let long = "a".repeat(81);
    assert!(matches!(Slug::parse(&long), Err(SlugError::TooLong(81))));
}

#[test]
fn parse_rejects_leading_hyphen() {
    assert!(matches!(Slug::parse("-foo"), Err(SlugError::LeadingHyphen)));
}

#[test]
fn parse_rejects_trailing_hyphen() {
    assert!(matches!(
        Slug::parse("foo-"),
        Err(SlugError::TrailingHyphen)
    ));
}

#[test]
fn parse_rejects_consecutive_hyphens() {
    assert!(matches!(
        Slug::parse("foo--bar"),
        Err(SlugError::ConsecutiveHyphens)
    ));
}

#[test]
fn parse_rejects_invalid_char() {
    assert!(matches!(
        Slug::parse("hello_world"),
        Err(SlugError::InvalidChar('_'))
    ));
    assert!(matches!(
        Slug::parse("hello world"),
        Err(SlugError::InvalidChar(' '))
    ));
}

#[test]
fn parse_accepts_max_length() {
    let s = "a".repeat(80);
    assert!(Slug::parse(&s).is_ok());
}

// ── allocate_slug edge cases ────────────────────────────────────────────────

#[test]
fn collision_with_long_base_slug_stays_within_limit() {
    // Max-length (80-char) base slug.
    let long_base = Slug::parse(&"a".repeat(80)).unwrap();
    let mut used = HashSet::from([long_base.as_str().to_string()]);
    let resolved = allocate_slug(&long_base, "https://example.com/conflict", &mut used);
    assert!(
        resolved.as_str().len() <= 80,
        "resolved slug too long: {} chars",
        resolved.as_str().len()
    );
    // Must be parseable after the fix.
    assert!(
        Slug::parse(resolved.as_str()).is_ok(),
        "resolved slug fails validation: {:?}",
        resolved.as_str()
    );
}

// ── intra-batch collision (ex allocate_slug_batch) ───────────────────────────

#[test]
fn intra_batch_collision_distinct_slugs() {
    let mut used = HashSet::new();
    let base = Slug::parse("introduction").unwrap();
    let r1 = allocate_slug(&base, "https://example.com/intro1", &mut used);
    let r2 = allocate_slug(&base, "https://example.com/intro2", &mut used);
    assert_eq!(r1.as_str(), "introduction");
    assert_ne!(r2.as_str(), "introduction");
    assert!(r2.as_str().starts_with("introduction-"));
    assert_ne!(r1, r2);
}

// ── Slug::generate ───────────────────────────────────────────────────────────

#[test]
fn generate_from_title() {
    let s = Slug::generate("Hello World", "https://example.com");
    assert_eq!(s.as_str(), "hello-world");
}

#[test]
fn generate_fallback_to_url() {
    let s = Slug::generate("", "https://example.com/some-page");
    assert!(!s.as_str().is_empty());
}

// ── Serde ────────────────────────────────────────────────────────────────────

#[test]
fn serialize_slug() {
    let s = Slug::parse("test-slug").unwrap();
    let json = serde_json::to_string(&s).unwrap();
    assert_eq!(json, "\"test-slug\"");
}

#[test]
fn deserialize_valid_slug() {
    let s: Slug = serde_json::from_str("\"valid-slug\"").unwrap();
    assert_eq!(s.as_str(), "valid-slug");
}

#[test]
fn deserialize_rejects_invalid_slug() {
    let result: Result<Slug, _> = serde_json::from_str("\"--invalid\"");
    assert!(result.is_err());
}
