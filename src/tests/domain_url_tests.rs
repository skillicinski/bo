use super::*;

// ── parse ──────────────────────────────────────────────────────────────

#[test]
fn parse_http() {
    let u = Url::parse("http://example.com/path").unwrap();
    assert_eq!(u.as_str(), "http://example.com/path");
}

#[test]
fn parse_https() {
    let u = Url::parse("https://example.com").unwrap();
    assert_eq!(u.as_str(), "https://example.com");
}

#[test]
fn parse_rejects_garbage() {
    let err = Url::parse("not a url").unwrap_err();
    assert!(matches!(err, UrlError::Invalid(_)));
}

#[test]
fn parse_rejects_scheme_less() {
    let err = Url::parse("example.com").unwrap_err();
    assert!(matches!(err, UrlError::Invalid(_)));
}

// ── no normalization ───────────────────────────────────────────────────

#[test]
fn stores_original_string_unchanged_no_normalization() {
    let input = "HTTPS://Example.COM/Path?a=b";
    let u = Url::parse(input).unwrap();
    assert_eq!(u.as_str(), input);
    // Serialize round-trip must preserve original exactly.
    let json = serde_json::to_string(&u).unwrap();
    assert_eq!(json, format!("\"{input}\""));
    let deser: Url = serde_json::from_str(&json).unwrap();
    assert_eq!(deser.as_str(), input);
}

// ── display ────────────────────────────────────────────────────────────

#[test]
fn display_renders_inner() {
    let u = Url::parse("https://x.com").unwrap();
    assert_eq!(format!("{}", u), "https://x.com");
}

// ── serde ──────────────────────────────────────────────────────────────

#[test]
fn serialize_as_string() {
    let u = Url::parse("https://test.dev").unwrap();
    let json = serde_json::to_string(&u).unwrap();
    assert_eq!(json, "\"https://test.dev\"");
}

#[test]
fn deserialize_from_string() {
    let u: Url = serde_json::from_str("\"https://foo.bar\"").unwrap();
    assert_eq!(u.as_str(), "https://foo.bar");
}

#[test]
fn deserialize_rejects_invalid() {
    let err = serde_json::from_str::<Url>("\"@@@\"").unwrap_err();
    assert!(err.to_string().contains("invalid URL"));
}

#[test]
fn roundtrip_serde() {
    let original = Url::parse("https://example.com/a?b=c").unwrap();
    let json = serde_json::to_string(&original).unwrap();
    let deserialized: Url = serde_json::from_str(&json).unwrap();
    assert_eq!(original, deserialized);
}

#[test]
fn clone_and_eq() {
    let a = Url::parse("https://example.com").unwrap();
    let b = a.clone();
    assert_eq!(a, b);
}
