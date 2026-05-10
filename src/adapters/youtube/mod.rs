mod innertube;
mod transcript;
mod url;

pub use url::{classify_url, SupportedYoutubeUrl, YoutubeUrlMatch};

use std::fmt;

#[derive(Debug)]
pub struct YoutubeTranscriptDocument {
    pub url: String,
    pub title: String,
    pub body_markdown: String,
}

#[derive(Debug)]
pub enum YoutubeError {
    UnsupportedUrl { url: String, reason: String },
    Network(String),
    Player(String),
    NoEnglishCaptions,
    EmptyTranscript,
    Parse(String),
}

impl fmt::Display for YoutubeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            YoutubeError::UnsupportedUrl { reason, .. } => {
                write!(f, "unsupported YouTube URL: {}", reason)
            }
            YoutubeError::Network(msg) => write!(f, "YouTube network error: {}", msg),
            YoutubeError::Player(msg) => write!(f, "YouTube player error: {}", msg),
            YoutubeError::NoEnglishCaptions => {
                write!(
                    f,
                    "YouTube transcript unavailable: no English captions found"
                )
            }
            YoutubeError::EmptyTranscript => {
                write!(f, "YouTube transcript unavailable: transcript is empty")
            }
            YoutubeError::Parse(msg) => write!(f, "YouTube transcript parse error: {}", msg),
        }
    }
}

pub fn collect_transcript(url: &str) -> Result<YoutubeTranscriptDocument, YoutubeError> {
    let supported = match classify_url(url) {
        YoutubeUrlMatch::Supported(supported) => supported,
        YoutubeUrlMatch::Unsupported { url, reason } => {
            return Err(YoutubeError::UnsupportedUrl { url, reason })
        }
        YoutubeUrlMatch::NotYoutube => {
            return Err(YoutubeError::UnsupportedUrl {
                url: url.to_string(),
                reason: "not a YouTube URL".to_string(),
            })
        }
    };

    fetch_supported_transcript(&supported)
}

pub fn fetch_supported_transcript(
    supported: &SupportedYoutubeUrl,
) -> Result<YoutubeTranscriptDocument, YoutubeError> {
    let player = innertube::fetch_player_response(supported.video_id())?;
    innertube::ensure_playable(&player)?;

    let title = player
        .video_details
        .as_ref()
        .and_then(|details| details.title.as_deref())
        .and_then(non_empty)
        .unwrap_or_else(|| supported.video_id().to_string());

    let track = innertube::select_english_caption_track(&player.caption_tracks())
        .ok_or(YoutubeError::NoEnglishCaptions)?;
    let xml = innertube::fetch_caption_xml(&track.base_url)?;
    let body_markdown = transcript::parse_transcript_markdown(&xml)?;

    Ok(YoutubeTranscriptDocument {
        url: supported.normalized_url().to_string(),
        title,
        body_markdown,
    })
}

fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

#[cfg(test)]
#[path = "../../tests/adapters_youtube_mod_tests.rs"]
mod network_tests;
