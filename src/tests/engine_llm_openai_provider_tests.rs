use super::*;

#[test]
fn provider_error_sanitizer_redacts_openai_key_fragments() {
    let message = "Incorrect API key provided: sk-dogfo*******-key. Check credentials.";

    let sanitized = sanitize_provider_error_message(message);

    assert!(!sanitized.contains("sk-dogfo"));
    assert!(!sanitized.contains("*******-key"));
    assert!(sanitized.contains("<redacted>"));
    assert!(sanitized.contains("Incorrect API key provided"));
}

#[test]
fn provider_error_sanitizer_redacts_key_like_json_tokens() {
    let message = r#"body: {"api_key":"sk-json-secret"}"#;

    let sanitized = sanitize_provider_error_message(message);

    assert!(!sanitized.contains("sk-json-secret"));
    assert!(sanitized.contains("<redacted>"));
}

#[test]
fn to_api_message_assistant_returns_error_not_panic() {
    let msg = Message {
        role: Role::Assistant,
        content: "test".to_string(),
    };
    let result = to_api_message(&msg);
    assert!(result.is_err());
}
