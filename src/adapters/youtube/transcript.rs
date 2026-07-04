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
            Ok(Event::Text(text)) if in_p || in_text => {
                let unescaped = text
                    .unescape()
                    .map_err(|e| YoutubeError::Parse(e.to_string()))?;
                current.push_str(&unescaped);
            }
            Ok(Event::CData(text)) if in_p || in_text => {
                let raw = String::from_utf8_lossy(text.as_ref());
                let decoded = quick_xml::escape::unescape(&raw)
                    .map_err(|e| YoutubeError::Parse(e.to_string()))?;
                current.push_str(&decoded);
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
    let mut text = input.to_string();
    loop {
        match quick_xml::escape::unescape(&text) {
            Ok(decoded) if decoded.as_ref() != text => text = decoded.into_owned(),
            _ => break,
        }
    }
    normalize_whitespace(&text)
}

fn normalize_whitespace(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
#[path = "../../tests/adapters_youtube_transcript_tests.rs"]
mod tests;
