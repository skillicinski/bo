use super::*;

#[test]
fn fallback_empty_body() {
    assert_eq!(generate_fallback(""), "");
}

#[test]
fn fallback_short_body_returned_as_is() {
    let body = "This is a short body with only a few words.";
    assert_eq!(generate_fallback(body), body);
}

#[test]
fn fallback_truncates_at_200_words() {
    let words: Vec<String> = (0..300).map(|i| format!("word{}", i)).collect();
    let body = words.join(" ");
    let result = generate_fallback(&body);
    assert_eq!(result.split_whitespace().count(), 200);
    assert!(result.starts_with("word0 word1"));
    assert!(result.ends_with("word199"));
}

#[test]
fn fallback_normalizes_whitespace() {
    let body = "hello   world\n\nnew  paragraph\there";
    let result = generate_fallback(body);
    assert_eq!(result, "hello world new paragraph here");
}

#[test]
fn fallback_exactly_200_words() {
    let words: Vec<String> = (0..200).map(|i| format!("w{}", i)).collect();
    let body = words.join(" ");
    let result = generate_fallback(&body);
    assert_eq!(result.split_whitespace().count(), 200);
}

#[test]
fn truncate_body_short_input_unchanged() {
    let body = "short text here";
    assert_eq!(truncate_body(body, 4000), "short text here");
}

#[test]
fn truncate_body_long_input_cut() {
    let words: Vec<String> = (0..5000).map(|i| format!("w{}", i)).collect();
    let body = words.join(" ");
    let result = truncate_body(&body, 4000);
    assert_eq!(result.split_whitespace().count(), 4000);
}
