use super::*;

#[test]
fn sanitizer_redacts_key_fragments() {
    let message = "Invalid API key: sk-dogfood-key-12345.";

    let sanitized = sanitize_error_message(message);

    assert!(!sanitized.contains("sk-dogfood"));
    assert!(sanitized.contains("<redacted>"));
}

#[test]
fn sanitizer_redacts_key_in_json_body() {
    let message = r#"body: {"api_key":"sk-json-secret-value"}"#;

    let sanitized = sanitize_error_message(message);

    assert!(!sanitized.contains("sk-json-secret-value"));
    assert!(sanitized.contains("<redacted>"));
}
