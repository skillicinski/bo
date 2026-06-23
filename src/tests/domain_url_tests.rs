use super::*;

#[test]
fn url_is_plain_string() {
    let u: Url = "https://example.com/path".to_string();
    assert_eq!(u.as_str(), "https://example.com/path");
}

#[test]
fn display_renders_inner() {
    let u: Url = "https://x.com".to_string();
    assert_eq!(format!("{}", u), "https://x.com");
}

#[test]
fn as_ref_str() {
    let u: Url = "https://example.com".to_string();
    let s: &str = u.as_ref();
    assert_eq!(s, "https://example.com");
}

#[test]
fn serialize_as_string() {
    let u: Url = "https://test.dev".to_string();
    let json = serde_json::to_string(&u).unwrap();
    assert_eq!(json, "\"https://test.dev\"");
}

#[test]
fn deserialize_from_string() {
    let u: Url = serde_json::from_str("\"https://foo.bar\"").unwrap();
    assert_eq!(u.as_str(), "https://foo.bar");
}

#[test]
fn roundtrip_serde() {
    let original: Url = "https://example.com/a?b=c".to_string();
    let json = serde_json::to_string(&original).unwrap();
    let deserialized: Url = serde_json::from_str(&json).unwrap();
    assert_eq!(original, deserialized);
}
