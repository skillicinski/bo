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
fn validate_structured_output_rejects_missing_required_field() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string" },
            "score": { "type": "number" }
        },
        "required": ["summary", "score"]
    });
    // Content JSON string missing the "score" field
    let content = serde_json::Value::String(r#"{"summary": "test summary"}"#.to_string());

    let result = validate_structured_output(&content, &schema);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(matches!(err, LlmError::Api(_)));
    assert!(err.to_string().contains("score"));
}

#[test]
fn validate_structured_output_accepts_complete_response() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "summary": { "type": "string" }
        },
        "required": ["summary"]
    });
    let content = serde_json::Value::String(r#"{"summary": "test summary"}"#.to_string());

    let result = validate_structured_output(&content, &schema);
    assert!(result.is_ok());
}
