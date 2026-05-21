use super::*;

#[test]
fn parse_valid_openai_model() {
    let m = Model::parse("gpt-4o", Provider::OpenAI).unwrap();
    assert_eq!(m.as_str(), "gpt-4o");
    assert_eq!(m.context_tokens(), 128_000);
}

#[test]
fn parse_valid_deepseek_model() {
    let m = Model::parse("deepseek-v4-flash", Provider::Deepseek).unwrap();
    assert_eq!(m.as_str(), "deepseek-v4-flash");
    assert_eq!(m.context_tokens(), 1_000_000);
}

#[test]
fn parse_trims_whitespace() {
    let m = Model::parse("  gpt-4.1-mini  ", Provider::OpenAI).unwrap();
    assert_eq!(m.as_str(), "gpt-4.1-mini");
}

#[test]
fn parse_unsupported_returns_error() {
    let err = Model::parse("gpt-5-turbo", Provider::OpenAI).unwrap_err();
    assert_eq!(err.id, "gpt-5-turbo");
    assert!(err.to_string().contains("unsupported model"));
    assert_eq!(err.provider, Provider::OpenAI);
}

#[test]
fn parse_openai_model_with_deepseek_provider_fails() {
    let err = Model::parse("gpt-4o", Provider::Deepseek).unwrap_err();
    assert_eq!(err.id, "gpt-4o");
    assert_eq!(err.provider, Provider::Deepseek);
    assert!(err.to_string().contains("unsupported model"));
    assert!(err.to_string().contains("deepseek"));
}

#[test]
fn default_model_is_gpt_4_1_mini() {
    let m = Model::default_model();
    assert_eq!(m.as_str(), "gpt-4.1-mini");
}

#[test]
fn context_tokens_are_correct() {
    let m = Model::parse("gpt-4.1", Provider::OpenAI).unwrap();
    assert_eq!(m.context_tokens(), 1_000_000);
}

#[test]
fn serialize_as_string() {
    let m = Model::parse("gpt-4o-mini", Provider::OpenAI).unwrap();
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
