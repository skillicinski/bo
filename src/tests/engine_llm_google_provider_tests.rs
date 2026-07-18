use super::*;

// ── schema dialect tests ────────────────────────────────────────────────────

#[test]
fn to_gemini_schema_strips_additional_properties_false() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" }
        },
        "additionalProperties": false
    });

    let result = to_gemini_schema(&schema).unwrap();

    assert_eq!(result["type"], "object");
    assert_eq!(result["properties"]["name"]["type"], "string");
    assert!(result.get("additionalProperties").is_none());
}

#[test]
fn to_gemini_schema_strips_nested_additional_properties() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "items": {
                "type": "array",
                "items": {
                    "type": "string",
                    "additionalProperties": false
                }
            }
        },
        "additionalProperties": false
    });

    let result = to_gemini_schema(&schema).unwrap();

    let inner = &result["properties"]["items"]["items"];
    assert!(inner.get("additionalProperties").is_none());
    assert_eq!(inner["type"], "string");
}

#[test]
fn to_gemini_schema_pass_through_clean_schema() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "answer": { "type": "string" }
        },
        "required": ["answer"]
    });

    let result = to_gemini_schema(&schema).unwrap();

    assert_eq!(result, schema);
}

#[test]
fn to_gemini_schema_rejects_additional_properties_object() {
    let schema = serde_json::json!({
        "type": "object",
        "additionalProperties": { "type": "string" }
    });

    let err = to_gemini_schema(&schema).unwrap_err();

    assert!(
        err.to_string().contains("additionalProperties"),
        "expected error about additionalProperties, got: {err}"
    );
}

#[test]
fn to_gemini_schema_rejects_additional_properties_true() {
    let schema = serde_json::json!({
        "type": "object",
        "additionalProperties": true
    });

    let err = to_gemini_schema(&schema).unwrap_err();

    assert!(
        err.to_string().contains("additionalProperties"),
        "expected error about additionalProperties, got: {err}"
    );
}

#[test]
fn to_gemini_schema_handles_arrays() {
    let schema = serde_json::json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": { "x": { "type": "number" } },
            "additionalProperties": false
        }
    });

    let result = to_gemini_schema(&schema).unwrap();

    assert_eq!(result["type"], "array");
    assert!(result["items"].get("additionalProperties").is_none());
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
        .complete(&messages, "gemini-flash-latest", 2000, None, false)
        .await;

    match result {
        Ok(response) => {
            assert!(
                !response.content.is_empty(),
                "expected non-empty response from Gemini"
            );
            assert_eq!(
                response.finish_reason,
                FinishReason::Stop,
                "expected STOP, got {:?}",
                response.finish_reason
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

    let normalized = provider.map_response_schema(&schema).unwrap();
    let result = provider
        .complete(
            &messages,
            "gemini-flash-latest",
            2000,
            Some(&normalized),
            false,
        )
        .await;

    match result {
        Ok(response) => {
            assert_eq!(
                response.finish_reason,
                FinishReason::Stop,
                "expected STOP, got {:?}",
                response.finish_reason
            );
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
