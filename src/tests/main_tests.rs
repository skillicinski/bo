use super::*;

#[test]
fn raw_json_mode_detection_stops_at_arg_terminator() {
    assert!(raw_json_mode_requested(&[
        OsString::from("bo"),
        OsString::from("search"),
        OsString::from("--json"),
    ]));
    assert!(!raw_json_mode_requested(&[
        OsString::from("bo"),
        OsString::from("search"),
        OsString::from("--"),
        OsString::from("--json"),
    ]));
}

#[test]
fn compile_validation_json_error_includes_next_action() {
    let error = compile_json_error(&CompileError::Validation(
        "invalid compile response".to_string(),
    ));

    assert_eq!(error.code, "validation_error");
    assert_eq!(error.message, "invalid compile response");
    assert_eq!(error.details["phase"], "compile_validation");
    assert_eq!(error.details["files_changed"], false);
    assert_eq!(error.details["next_step"], compile::VALIDATION_NEXT_STEP);
}
