use quick_xml::events::Event;
use quick_xml::Reader;

use super::YoutubeError;

pub fn parse_transcript_markdown(xml: &str) -> Result<String, YoutubeError> {
    let segments = parse_segments(xml)?;
    let body = segments
        .into_iter()
        .map(|segment| clean_text(&segment))
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");

    if body.trim().is_empty() {
        return Err(YoutubeError::EmptyTranscript);
    }
    Ok(body)
}

fn parse_segments(xml: &str) -> Result<Vec<String>, YoutubeError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(false);

    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_p = false;
    let mut in_text = false;
    let mut saw_timedtext_p = false;
    let mut saw_simple_text = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => match event.name().as_ref() {
                b"p" => {
                    in_p = true;
                    saw_timedtext_p = true;
                    current.clear();
                }
                b"text" => {
                    in_text = true;
                    saw_simple_text = true;
                    current.clear();
                }
                _ => {}
            },
            Ok(Event::Text(text)) => {
                if in_p || in_text {
                    let unescaped = text
                        .unescape()
                        .map_err(|e| YoutubeError::Parse(e.to_string()))?;
                    current.push_str(&unescaped);
                }
            }
            Ok(Event::CData(text)) => {
                if in_p || in_text {
                    let content = String::from_utf8_lossy(text.as_ref());
                    current.push_str(&content);
                }
            }
            Ok(Event::End(event)) => match event.name().as_ref() {
                b"p" if in_p => {
                    push_current(&mut segments, &mut current);
                    in_p = false;
                }
                b"text" if in_text => {
                    push_current(&mut segments, &mut current);
                    in_text = false;
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(YoutubeError::Parse(e.to_string())),
            _ => {}
        }
    }

    if !saw_timedtext_p && !saw_simple_text {
        return Err(YoutubeError::EmptyTranscript);
    }

    Ok(segments)
}

fn push_current(segments: &mut Vec<String>, current: &mut String) {
    let cleaned = clean_text(current);
    if !cleaned.is_empty() {
        segments.push(cleaned);
    }
    current.clear();
}

fn clean_text(input: &str) -> String {
    let once = decode_common_entities(input);
    let twice = decode_common_entities(&once);
    normalize_whitespace(&twice)
}

fn normalize_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_common_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '&' {
            output.push(ch);
            continue;
        }

        let mut entity = String::new();
        while let Some(&next) = chars.peek() {
            entity.push(next);
            chars.next();
            if next == ';' || entity.len() > 16 {
                break;
            }
        }

        let decoded = match entity.as_str() {
            "amp;" => Some('&'),
            "lt;" => Some('<'),
            "gt;" => Some('>'),
            "quot;" => Some('"'),
            "apos;" => Some('\''),
            "#39;" => Some('\''),
            _ => decode_numeric_entity(&entity),
        };

        if let Some(decoded) = decoded {
            output.push(decoded);
        } else {
            output.push('&');
            output.push_str(&entity);
        }
    }

    output
}

fn decode_numeric_entity(entity: &str) -> Option<char> {
    let value = entity.strip_suffix(';')?;
    let codepoint = if let Some(hex) = value
        .strip_prefix("#x")
        .or_else(|| value.strip_prefix("#X"))
    {
        u32::from_str_radix(hex, 16).ok()?
    } else if let Some(decimal) = value.strip_prefix('#') {
        decimal.parse::<u32>().ok()?
    } else {
        return None;
    };
    char::from_u32(codepoint)
}

#[cfg(test)]
mod tests {
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
}
