use super::*;

#[test]
fn sanitizer_redacts_key_fragments() {
    let message = "Invalid API key: sk-dogfood-key-12345.";

    let sanitized = sanitize_provider_error_message(message);

    assert!(!sanitized.contains("sk-dogfood"));
    assert!(sanitized.contains("<redacted>"));
}

#[test]
fn sanitizer_redacts_key_in_json_body() {
    let message = r#"body: {"api_key":"sk-json-secret-value"}"#;

    let sanitized = sanitize_provider_error_message(message);

    assert!(!sanitized.contains("sk-json-secret-value"));
    assert!(sanitized.contains("<redacted>"));
}

#[test]
fn custom_constructor_appends_chat_completions() {
    let provider = OpenAiCompatProvider::custom("sk-test", "https://api.example.com/v1");
    assert_eq!(
        provider.base_url,
        "https://api.example.com/v1/chat/completions"
    );
}

#[test]
fn custom_constructor_trims_trailing_slash() {
    let provider = OpenAiCompatProvider::custom("sk-test", "https://api.example.com/v1/");
    assert_eq!(
        provider.base_url,
        "https://api.example.com/v1/chat/completions"
    );
}
