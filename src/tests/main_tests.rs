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
