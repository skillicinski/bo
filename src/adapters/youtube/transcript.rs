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
#[path = "../../tests/adapters_youtube_transcript_tests.rs"]
mod tests;
