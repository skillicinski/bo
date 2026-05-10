use super::*;

#[derive(Serialize)]
struct Payload {
    name: &'static str,
}

#[test]
fn success_envelope_has_required_top_level_fields() {
    let encoded = success_string(
        "list",
        Payload { name: "example" },
        vec![JsonWarning::new("note", "be careful")],
    )
    .unwrap();
    let parsed: Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(parsed["schema_version"], SCHEMA_VERSION);
    assert_eq!(parsed["ok"], true);
    assert_eq!(parsed["command"], "list");
    assert_eq!(parsed["data"]["name"], "example");
    assert_eq!(parsed["warnings"][0]["code"], "note");
    assert!(parsed.get("error").is_none());
}

#[test]
fn error_envelope_has_required_top_level_fields() {
    let encoded = error_string(
        "show",
        JsonError::with_details("not_found", "missing", json!({ "title": "x" })),
        Vec::new(),
    )
    .unwrap();
    let parsed: Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(parsed["schema_version"], SCHEMA_VERSION);
    assert_eq!(parsed["ok"], false);
    assert_eq!(parsed["command"], "show");
    assert_eq!(parsed["error"]["code"], "not_found");
    assert_eq!(parsed["error"]["details"]["title"], "x");
    assert_eq!(parsed["warnings"].as_array().unwrap().len(), 0);
    assert!(parsed.get("data").is_none());
}

#[test]
fn non_object_details_are_normalized_to_empty_object() {
    let error = JsonError::with_details("bad", "bad", json!("not-object"));
    assert_eq!(error.details, json!({}));

    let warning = JsonWarning::with_details("bad", "bad", json!(null));
    assert_eq!(warning.details, json!({}));
}
