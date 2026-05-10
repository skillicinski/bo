use super::*;

#[test]
fn parses_timedtext_p_s_segments() {
    let xml = r#"<?xml version="1.0"?><timedtext><body>
<p t="0" d="1000"><s>Hello</s><s t="10"> world</s><s> &amp; friends</s></p>
<p t="1000" d="1000"><s>Second</s><s> segment.</s></p>
</body></timedtext>"#;

    let body = parse_transcript_markdown(xml).unwrap();
    assert_eq!(body, "Hello world & friends\n\nSecond segment.");
}

#[test]
fn parses_timedtext_text_directly_inside_p() {
    let xml = r#"<timedtext><body><p t="0">Hello <b>styled</b> world</p></body></timedtext>"#;

    let body = parse_transcript_markdown(xml).unwrap();
    assert_eq!(body, "Hello styled world");
}

#[test]
fn parses_simple_transcript_text_nodes() {
    let xml = r#"<transcript>
<text start="0" dur="1.0">Hello &amp; welcome</text>
<text start="1" dur="1.0">to the show</text>
</transcript>"#;

    let body = parse_transcript_markdown(xml).unwrap();
    assert_eq!(body, "Hello & welcome\n\nto the show");
}

#[test]
fn decodes_double_encoded_entities() {
    let xml = r#"<transcript><text start="0">I&amp;amp;#39;m &amp;quot;here&amp;quot;</text></transcript>"#;

    let body = parse_transcript_markdown(xml).unwrap();
    assert_eq!(body, "I'm \"here\"");
}

#[test]
fn skips_blank_layout_paragraphs() {
    let xml = r#"<timedtext><body>
<p t="0" d="1000"><s>Hello</s></p>
<p t="1000" d="1000">
</p>
<p t="2000" d="1000"><s>world</s></p>
</body></timedtext>"#;

    let body = parse_transcript_markdown(xml).unwrap();
    assert_eq!(body, "Hello\n\nworld");
}

#[test]
fn rejects_empty_transcript() {
    let xml = r#"<timedtext><body><p t="0"></p></body></timedtext>"#;
    assert!(matches!(
        parse_transcript_markdown(xml),
        Err(YoutubeError::EmptyTranscript)
    ));
}

#[test]
fn rejects_valid_xml_with_no_transcript_nodes() {
    let xml = r#"<root><title>No transcript</title></root>"#;
    assert!(matches!(
        parse_transcript_markdown(xml),
        Err(YoutubeError::EmptyTranscript)
    ));
}

#[test]
fn rejects_malformed_xml() {
    let xml = r#"<timedtext><body><p>Hello</body></timedtext>"#;
    assert!(matches!(
        parse_transcript_markdown(xml),
        Err(YoutubeError::Parse(_))
    ));
}

#[test]
fn output_has_no_timestamps_or_links() {
    let xml = r#"<transcript><text start="0">Hello world</text></transcript>"#;
    let body = parse_transcript_markdown(xml).unwrap();
    assert!(!body.contains("0:00"));
    assert!(!body.contains("]("));
}
