use super::*;

#[test]
fn parse_valid_model() {
    let m = Model::parse("gpt-4o").unwrap();
    assert_eq!(m.as_str(), "gpt-4o");
    assert_eq!(m.context_tokens(), 128_000);
}

#[test]
fn parse_trims_whitespace() {
    let m = Model::parse("  gpt-4.1-mini  ").unwrap();
    assert_eq!(m.as_str(), "gpt-4.1-mini");
}

#[test]
fn parse_unsupported_returns_error() {
    let err = Model::parse("gpt-5-turbo").unwrap_err();
    assert_eq!(err.id, "gpt-5-turbo");
    assert!(err.to_string().contains("unsupported model"));
}

#[test]
fn default_model_is_gpt4o() {
    let m = Model::default_model();
    assert_eq!(m.as_str(), "gpt-4o");
}

#[test]
fn context_tokens_are_correct() {
    let m = Model::parse("gpt-4.1").unwrap();
    assert_eq!(m.context_tokens(), 1_000_000);
}

#[test]
fn serialize_as_string() {
    let m = Model::parse("gpt-4o-mini").unwrap();
    let json = serde_json::to_string(&m).unwrap();
    assert_eq!(json, "\"gpt-4o-mini\"");
}

#[test]
fn deserialize_from_string() {
    let m: Model = serde_json::from_str("\"gpt-4.1-nano\"").unwrap();
    assert_eq!(m.as_str(), "gpt-4.1-nano");
}

#[test]
fn deserialize_invalid_is_error() {
    let result: Result<Model, _> = serde_json::from_str("\"not-a-model\"");
    assert!(result.is_err());
}
