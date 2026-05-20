use super::*;

#[test]
fn parse_valid_url() {
    let u = Url::parse("https://example.com/path").unwrap();
    assert_eq!(u.as_str(), "https://example.com/path");
}

#[test]
fn parse_http_url() {
    let u = Url::parse("http://example.com").unwrap();
    assert_eq!(u.as_str(), "http://example.com");
}

#[test]
fn parse_rejects_empty() {
    assert_eq!(Url::parse(""), Err(UrlError::Empty));
}

#[test]
fn parse_rejects_no_scheme() {
    assert_eq!(Url::parse("example.com/path"), Err(UrlError::NoScheme));
}

#[test]
fn display_renders_inner() {
    let u = Url::parse("https://x.com").unwrap();
    assert_eq!(format!("{}", u), "https://x.com");
}

#[test]
fn as_ref_str() {
    let u = Url::parse("https://example.com").unwrap();
    let s: &str = u.as_ref();
    assert_eq!(s, "https://example.com");
}

#[test]
fn serialize_as_string() {
    let u = Url::parse("https://test.dev").unwrap();
    let json = serde_json::to_string(&u).unwrap();
    assert_eq!(json, "\"https://test.dev\"");
}

#[test]
fn deserialize_valid() {
    let u: Url = serde_json::from_str("\"https://foo.bar\"").unwrap();
    assert_eq!(u.as_str(), "https://foo.bar");
}

#[test]
fn deserialize_rejects_invalid() {
    let result: Result<Url, _> = serde_json::from_str("\"no-scheme\"");
    assert!(result.is_err());
}

#[test]
fn roundtrip_serde() {
    let original = Url::parse("https://example.com/a?b=c").unwrap();
    let json = serde_json::to_string(&original).unwrap();
    let deserialized: Url = serde_json::from_str(&json).unwrap();
    assert_eq!(original, deserialized);
}

#[test]
fn various_schemes_accepted() {
    assert!(Url::parse("ftp://files.example.com").is_ok());
    assert!(Url::parse("file:///tmp/test").is_ok());
}
