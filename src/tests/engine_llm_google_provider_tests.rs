use super::*;

#[test]
fn sanitizer_redacts_gemini_key_fragments() {
    let message = "API key not valid: AIzaSyD-test-key-12345.";

    let sanitized = sanitize_provider_error_message(message);

    assert!(!sanitized.contains("AIzaSyD-test-key-12345"));
}

#[test]
fn sanitizer_redacts_key_in_json_body() {
    let message = r#"body: {"api_key":"AIzaSySecretKey"}"#;

    let sanitized = sanitize_provider_error_message(message);

    assert!(!sanitized.contains("AIzaSySecretKey"));
}

// ── smoke tests (require GEMINI_API_KEY env var) ──────────────────────────

#[tokio::test]
async fn google_provider_smoke_text_only() {
    let api_key = match std::env::var("GEMINI_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!("SKIP: GEMINI_API_KEY not set");
            return;
        }
    };

    let provider = GoogleProvider::new(&api_key);
    let messages = vec![
        Message::system("Keep answers extremely concise. One sentence max."),
        Message::user("What is the capital of France?"),
    ];

    let result = provider
        .complete(&messages, "gemini-2.5-flash", 100, None, false)
        .await;

    match result {
        Ok(response) => {
            assert!(
                !response.content.is_empty(),
                "expected non-empty response from Gemini"
            );
        }
        Err(e) => panic!("Gemini API call failed: {:?}", e),
    }
}

#[tokio::test]
async fn google_provider_smoke_structured_output() {
    let api_key = match std::env::var("GEMINI_API_KEY") {
        Ok(k) if !k.is_empty() => k,
        _ => {
            eprintln!("SKIP: GEMINI_API_KEY not set");
            return;
        }
    };

    let provider = GoogleProvider::new(&api_key);
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "answer": {"type": "string"},
            "confidence": {"type": "number"}
        },
        "required": ["answer", "confidence"]
    });

    let messages = vec![Message::user("What is 2+2?")];

    let result = provider
        .complete(&messages, "gemini-2.5-flash", 200, Some(&schema), false)
        .await;

    match result {
        Ok(response) => {
            let parsed: serde_json::Value =
                serde_json::from_str(&response.content).expect("response should be valid JSON");
            assert!(parsed.get("answer").is_some(), "missing 'answer' field");
            assert!(
                parsed.get("confidence").is_some(),
                "missing 'confidence' field"
            );
        }
        Err(e) => panic!("Gemini structured output call failed: {:?}", e),
    }
}
