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

pub fn collect_transcript(
    supported: &SupportedYoutubeUrl,
) -> Result<YoutubeTranscriptDocument, YoutubeError> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| YoutubeError::Network(e.to_string()))?;
    let player = innertube::fetch_player_response(&client, supported.video_id())?;
    innertube::ensure_playable(&player)?;

    let title = player
        .video_details
        .as_ref()
        .and_then(|details| details.title.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| supported.video_id().to_string());
    // Normalize smart quotes that YouTube occasionally surfaces in
    // auto-generated or translated video titles.
    let title = title
        .replace(['\u{2018}', '\u{2019}'], "'")
        .replace(['\u{201c}', '\u{201d}'], "\"");

    let tracks = player
        .captions
        .as_ref()
        .and_then(|c| c.player_captions_tracklist_renderer.as_ref())
        .map(|r| r.caption_tracks.as_slice())
        .unwrap_or(&[]);
    let track =
        innertube::select_english_caption_track(tracks).ok_or(YoutubeError::NoEnglishCaptions)?;
    let xml = innertube::fetch_caption_xml(&client, &track.base_url)?;
    let body_markdown = transcript::parse_transcript_markdown(&xml)?;

    Ok(YoutubeTranscriptDocument {
        url: supported.normalized_url().to_string(),
        title,
        body_markdown,
    })
}

#[cfg(test)]
#[path = "../../tests/adapters_youtube_mod_tests.rs"]
mod network_tests;
