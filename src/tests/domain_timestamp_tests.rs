use super::*;

#[test]
fn parse_valid_rfc3339() {
    let ts = Timestamp::parse("2024-01-01T00:00:00Z").unwrap();
    assert_eq!(ts.to_rfc3339_millis(), "2024-01-01T00:00:00.000Z");
}

#[test]
fn parse_with_milliseconds() {
    let ts = Timestamp::parse("2024-06-15T10:30:45.123Z").unwrap();
    assert_eq!(ts.to_rfc3339_millis(), "2024-06-15T10:30:45.123Z");
}

#[test]
fn parse_with_offset() {
    let ts = Timestamp::parse("2024-01-01T12:00:00+05:00").unwrap();
    // Converts to UTC
    assert_eq!(ts.to_rfc3339_millis(), "2024-01-01T07:00:00.000Z");
}

#[test]
fn parse_rejects_invalid() {
    assert!(Timestamp::parse("not-a-timestamp").is_err());
    assert!(Timestamp::parse("").is_err());
    assert!(Timestamp::parse("2024-13-01T00:00:00Z").is_err());
}

#[test]
fn now_produces_valid_timestamp() {
    let ts = Timestamp::now();
    let s = ts.to_rfc3339_millis();
    assert!(s.ends_with('Z'));
    // Roundtrip: parse the millis string back
    let parsed = Timestamp::parse(&s).unwrap();
    assert_eq!(ts.to_rfc3339_millis(), parsed.to_rfc3339_millis());
}

#[test]
fn display_is_rfc3339_millis() {
    let ts = Timestamp::parse("2024-01-01T00:00:00Z").unwrap();
    assert_eq!(format!("{}", ts), "2024-01-01T00:00:00.000Z");
}

#[test]
fn ord_ordering() {
    let earlier = Timestamp::parse("2024-01-01T00:00:00Z").unwrap();
    let later = Timestamp::parse("2024-06-15T12:00:00Z").unwrap();
    assert!(earlier < later);
    assert!(later > earlier);
    assert_eq!(earlier.cmp(&earlier), std::cmp::Ordering::Equal);
}

#[test]
fn serialize_as_rfc3339_string() {
    let ts = Timestamp::parse("2024-03-15T08:30:00.500Z").unwrap();
    let json = serde_json::to_string(&ts).unwrap();
    assert_eq!(json, "\"2024-03-15T08:30:00.500Z\"");
}

#[test]
fn deserialize_valid() {
    let ts: Timestamp = serde_json::from_str("\"2024-01-01T00:00:00Z\"").unwrap();
    assert_eq!(ts.to_rfc3339_millis(), "2024-01-01T00:00:00.000Z");
}

#[test]
fn deserialize_rejects_invalid() {
    let result: Result<Timestamp, _> = serde_json::from_str("\"nope\"");
    assert!(result.is_err());
}

#[test]
fn roundtrip_serde() {
    let original = Timestamp::parse("2024-12-31T23:59:59.999Z").unwrap();
    let json = serde_json::to_string(&original).unwrap();
    let deserialized: Timestamp = serde_json::from_str(&json).unwrap();
    assert_eq!(original, deserialized);
}

#[test]
fn as_datetime_returns_inner() {
    let ts = Timestamp::parse("2024-01-01T00:00:00Z").unwrap();
    let dt = ts.as_datetime();
    assert_eq!(dt.timestamp(), 1704067200);
}
