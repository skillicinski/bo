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
fn parse_valid_google_model() {
    let m = Model::parse("gemini-3.5-flash", Provider::Google).unwrap();
    assert_eq!(m.as_str(), "gemini-3.5-flash");
    assert_eq!(m.context_tokens(), 1_048_576);
}

#[test]
fn parse_unknown_google_model_falls_back() {
    let m = Model::parse("gemini-4-future", Provider::Google).unwrap();
    assert_eq!(m.as_str(), "gemini-4-future");
    assert_eq!(m.context_tokens(), models::GOOGLE_CONTEXT_TOKENS);
}

#[test]
fn parse_google_rejects_empty_model() {
    let err = Model::parse("   ", Provider::Google).unwrap_err();
    assert_eq!(err.id, "");
    assert_eq!(err.provider, Provider::Google);
}

#[test]
fn parse_valid_zai_model() {
    let m = Model::parse("glm-4.7", Provider::Zai).unwrap();
    assert_eq!(m.as_str(), "glm-4.7");
    assert_eq!(m.context_tokens(), 200_000);
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
fn parse_google_model_with_openai_provider_fails() {
    let err = Model::parse("gemini-3.5-flash", Provider::OpenAI).unwrap_err();
    assert_eq!(err.id, "gemini-3.5-flash");
    assert_eq!(err.provider, Provider::OpenAI);
    assert!(err.to_string().contains("unsupported model"));
    assert!(err.to_string().contains("openai"));
}

#[test]
fn context_tokens_are_correct() {
    let m = Model::parse("gpt-4.1", Provider::OpenAI).unwrap();
    assert_eq!(m.context_tokens(), 1_000_000);
}

#[test]
fn parse_custom_accepts_any_non_empty_model() {
    let m = Model::parse("  my-fine-tune  ", Provider::Custom).unwrap();
    assert_eq!(m.as_str(), "my-fine-tune");
    assert_eq!(m.context_tokens(), models::CUSTOM_CONTEXT_TOKENS);
}

#[test]
fn parse_custom_rejects_empty_model() {
    let err = Model::parse("   ", Provider::Custom).unwrap_err();
    assert_eq!(err.id, "");
    assert_eq!(err.provider, Provider::Custom);
}
