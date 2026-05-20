use super::*;

#[test]
fn new_accepts_any_string() {
    let t = Title::new("Hello World");
    assert_eq!(t.as_str(), "Hello World");
}

#[test]
fn new_accepts_empty_string() {
    let t = Title::new("");
    assert_eq!(t.as_str(), "");
}

#[test]
fn display_renders_inner() {
    let t = Title::new("Test Title");
    assert_eq!(format!("{}", t), "Test Title");
}

#[test]
fn as_ref_str() {
    let t = Title::new("ref test");
    let s: &str = t.as_ref();
    assert_eq!(s, "ref test");
}

#[test]
fn serialize_as_string() {
    let t = Title::new("serde test");
    let json = serde_json::to_string(&t).unwrap();
    assert_eq!(json, "\"serde test\"");
}

#[test]
fn deserialize_from_string() {
    let t: Title = serde_json::from_str("\"hello\"").unwrap();
    assert_eq!(t.as_str(), "hello");
}

#[test]
fn roundtrip_serde() {
    let original = Title::new("round trip");
    let json = serde_json::to_string(&original).unwrap();
    let deserialized: Title = serde_json::from_str(&json).unwrap();
    assert_eq!(original, deserialized);
}

#[test]
fn clone_and_eq() {
    let a = Title::new("clone");
    let b = a.clone();
    assert_eq!(a, b);
}
