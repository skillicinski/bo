use super::*;
use crate::cli::json;

#[test]
fn compile_result_notifications_skipped_from_json() {
    let result = CompileResult {
        status: "compiled".to_string(),
        reason: None,
        mode: Some(CompileRunMode::Full),
        model: Some("gpt-4.1".to_string()),
        branches: vec![BranchResult {
            slug: "test-branch".to_string(),
            title: "Test Branch".to_string(),
            leaf_count: 2,
        }],
        leaves_processed: 2,
        leaves_skipped: Vec::new(),
        notifications: vec!["pruned 3 orphan leaf records".to_string()],
        warnings: Vec::new(),
    };

    let encoded = json::success_string("compile", &result, Vec::new()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    assert_eq!(parsed["schema_version"], 1);
    assert_eq!(parsed["ok"], true);
    assert!(parsed["data"]["notifications"].is_null());
}

#[test]
fn compile_result_warnings_skipped_from_json() {
    let result = CompileResult {
        status: "compiled".to_string(),
        reason: None,
        mode: Some(CompileRunMode::Full),
        model: Some("gpt-4.1".to_string()),
        branches: vec![BranchResult {
            slug: "test-branch".to_string(),
            title: "Test Branch".to_string(),
            leaf_count: 2,
        }],
        leaves_processed: 2,
        leaves_skipped: Vec::new(),
        notifications: Vec::new(),
        warnings: vec!["warning: title collision — shared".to_string()],
    };

    let encoded = json::success_string("compile", &result, Vec::new()).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&encoded).unwrap();

    // warnings are presentation (stderr), never part of the JSON envelope.
    assert!(parsed["data"]["warnings"].is_null());
}
