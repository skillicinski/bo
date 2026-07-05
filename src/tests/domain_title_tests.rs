use super::*;

// ── parse ──────────────────────────────────────────────────────────────

#[test]
fn parse_valid() {
    let t = Title::parse("Hello World").unwrap();
    assert_eq!(t.as_str(), "Hello World");
}

#[test]
fn parse_trims_whitespace() {
    let t = Title::parse("  padded  ").unwrap();
    assert_eq!(t.as_str(), "padded");
}

#[test]
fn parse_rejects_empty() {
    let err = Title::parse("").unwrap_err();
    assert!(matches!(err, TitleError::Empty));
}

#[test]
fn parse_rejects_whitespace_only() {
    let err = Title::parse("   \t\n ").unwrap_err();
    assert!(matches!(err, TitleError::Empty));
}

// ── display ────────────────────────────────────────────────────────────

#[test]
fn display_renders_inner() {
    let t = Title::parse("Test Title").unwrap();
    assert_eq!(format!("{}", t), "Test Title");
}

// ── serde ──────────────────────────────────────────────────────────────

#[test]
fn serialize_as_string() {
    let t = Title::parse("serde test").unwrap();
    let json = serde_json::to_string(&t).unwrap();
    assert_eq!(json, "\"serde test\"");
}

#[test]
fn deserialize_from_string() {
    let t: Title = serde_json::from_str("\"hello\"").unwrap();
    assert_eq!(t.as_str(), "hello");
}

#[test]
fn deserialize_rejects_empty() {
    let err = serde_json::from_str::<Title>("\"\"").unwrap_err();
    assert!(err.to_string().contains("title cannot be empty"));
}

#[test]
fn roundtrip_serde() {
    let original = Title::parse("round trip").unwrap();
    let json = serde_json::to_string(&original).unwrap();
    let deserialized: Title = serde_json::from_str(&json).unwrap();
    assert_eq!(original, deserialized);
}

#[test]
fn clone_and_eq() {
    let a = Title::parse("clone").unwrap();
    let b = a.clone();
    assert_eq!(a, b);
}

// ── option migration helper ────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TestPayload {
    #[serde(default, deserialize_with = "deserialize_option_title")]
    pub title: Option<Title>,
}

#[test]
fn deserialize_option_title_missing_field_is_none() {
    let p: TestPayload = serde_json::from_str("{}").unwrap();
    assert!(p.title.is_none());
}

#[test]
fn deserialize_option_title_null_is_none() {
    let p: TestPayload = serde_json::from_str(r#"{"title":null}"#).unwrap();
    assert!(p.title.is_none());
}

#[test]
fn deserialize_option_title_empty_string_is_none() {
    let p: TestPayload = serde_json::from_str(r#"{"title":""}"#).unwrap();
    assert!(p.title.is_none());
}

#[test]
fn deserialize_option_title_whitespace_only_is_none() {
    let p: TestPayload = serde_json::from_str(r#"{"title":"   "}"#).unwrap();
    assert!(p.title.is_none());
}

#[test]
fn deserialize_option_title_valid_is_some() {
    let p: TestPayload = serde_json::from_str(r#"{"title":"X"}"#).unwrap();
    assert_eq!(p.title.as_ref().map(|t| t.as_str()), Some("X"));
}

#[test]
fn deserialize_option_title_trims_on_read() {
    let p: TestPayload = serde_json::from_str(r#"{"title":"  Hello  "}"#).unwrap();
    assert_eq!(p.title.as_ref().map(|t| t.as_str()), Some("Hello"));
}
