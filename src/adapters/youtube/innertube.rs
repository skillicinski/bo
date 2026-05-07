use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::YoutubeError;

const PLAYER_ENDPOINT: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
const ANDROID_CLIENT_NAME: &str = "ANDROID";
const ANDROID_CLIENT_VERSION: &str = "20.10.38";
const ANDROID_USER_AGENT: &str = "com.google.android.youtube/20.10.38 (Linux; U; Android 14)";
const TIMEOUT_SECONDS: u64 = 30;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerResponse {
    pub playability_status: Option<PlayabilityStatus>,
    pub video_details: Option<VideoDetails>,
    pub captions: Option<Captions>,
}

impl PlayerResponse {
    pub fn caption_tracks(&self) -> Vec<CaptionTrack> {
        self.captions
            .as_ref()
            .and_then(|captions| captions.player_captions_tracklist_renderer.as_ref())
            .map(|renderer| renderer.caption_tracks.clone())
            .unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayabilityStatus {
    pub status: Option<String>,
    pub reason: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoDetails {
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Captions {
    pub player_captions_tracklist_renderer: Option<PlayerCaptionsTracklistRenderer>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerCaptionsTracklistRenderer {
    pub caption_tracks: Vec<CaptionTrack>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CaptionTrack {
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub language_code: String,
    pub kind: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayerRequest<'a> {
    video_id: &'a str,
    context: PlayerContext<'a>,
}

#[derive(Serialize)]
struct PlayerContext<'a> {
    client: PlayerClient<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PlayerClient<'a> {
    client_name: &'a str,
    client_version: &'a str,
    hl: &'a str,
}

pub fn fetch_player_response(video_id: &str) -> Result<PlayerResponse, YoutubeError> {
    let client = build_client()?;
    let request = PlayerRequest {
        video_id,
        context: PlayerContext {
            client: PlayerClient {
                client_name: ANDROID_CLIENT_NAME,
                client_version: ANDROID_CLIENT_VERSION,
                hl: "en",
            },
        },
    };

    let response = client
        .post(PLAYER_ENDPOINT)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, ANDROID_USER_AGENT)
        .json(&request)
        .send()
        .map_err(|e| YoutubeError::Network(e.to_string()))?;

    if !response.status().is_success() {
        return Err(YoutubeError::Player(format!(
            "InnerTube returned HTTP {}",
            response.status().as_u16()
        )));
    }

    response
        .json::<PlayerResponse>()
        .map_err(|e| YoutubeError::Player(format!("invalid InnerTube response: {e}")))
}

pub fn fetch_caption_xml(base_url: &str) -> Result<String, YoutubeError> {
    let client = build_client()?;
    let response = client
        .get(base_url)
        .header(reqwest::header::USER_AGENT, ANDROID_USER_AGENT)
        .send()
        .map_err(|e| YoutubeError::Network(e.to_string()))?;

    if !response.status().is_success() {
        return Err(YoutubeError::Network(format!(
            "caption fetch returned HTTP {}",
            response.status().as_u16()
        )));
    }

    let xml = response
        .text()
        .map_err(|e| YoutubeError::Network(e.to_string()))?;
    if xml.trim().is_empty() {
        return Err(YoutubeError::EmptyTranscript);
    }
    Ok(xml)
}

pub fn ensure_playable(player: &PlayerResponse) -> Result<(), YoutubeError> {
    let Some(status) = &player.playability_status else {
        return Err(YoutubeError::Player(
            "missing playability status".to_string(),
        ));
    };
    match status.status.as_deref() {
        Some("OK") => Ok(()),
        Some(other) => {
            Err(YoutubeError::Player(status.reason.clone().unwrap_or_else(
                || format!("playability status is {other}"),
            )))
        }
        None => Err(YoutubeError::Player(
            "missing playability status".to_string(),
        )),
    }
}

pub fn select_english_caption_track(tracks: &[CaptionTrack]) -> Option<CaptionTrack> {
    tracks
        .iter()
        .find(|track| is_english(track) && !is_generated(track) && !track.base_url.is_empty())
        .or_else(|| {
            tracks.iter().find(|track| {
                is_english(track) && is_generated(track) && !track.base_url.is_empty()
            })
        })
        .cloned()
}

fn build_client() -> Result<Client, YoutubeError> {
    Client::builder()
        .timeout(Duration::from_secs(TIMEOUT_SECONDS))
        .build()
        .map_err(|e| YoutubeError::Network(e.to_string()))
}

fn is_english(track: &CaptionTrack) -> bool {
    track.language_code == "en" || track.language_code.starts_with("en-")
}

fn is_generated(track: &CaptionTrack) -> bool {
    track.kind.as_deref() == Some("asr")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(language_code: &str, kind: Option<&str>, base_url: &str) -> CaptionTrack {
        CaptionTrack {
            base_url: base_url.to_string(),
            language_code: language_code.to_string(),
            kind: kind.map(str::to_string),
        }
    }

    #[test]
    fn selects_manual_english_before_generated_english() {
        let tracks = vec![
            track("en", Some("asr"), "https://generated"),
            track("en", None, "https://manual"),
        ];

        let selected = select_english_caption_track(&tracks).unwrap();
        assert_eq!(selected.base_url, "https://manual");
    }

    #[test]
    fn accepts_generated_english() {
        let tracks = vec![track("en", Some("asr"), "https://generated")];

        let selected = select_english_caption_track(&tracks).unwrap();
        assert_eq!(selected.base_url, "https://generated");
    }

    #[test]
    fn accepts_regional_english() {
        let tracks = vec![track("en-GB", None, "https://manual")];

        let selected = select_english_caption_track(&tracks).unwrap();
        assert_eq!(selected.base_url, "https://manual");
    }

    #[test]
    fn never_selects_non_english() {
        let tracks = vec![
            track("fr", None, "https://fr"),
            track("es", Some("asr"), "https://es"),
        ];

        assert!(select_english_caption_track(&tracks).is_none());
    }

    #[test]
    fn skips_english_tracks_without_base_url() {
        let tracks = vec![track("en", None, ""), track("fr", None, "https://fr")];

        assert!(select_english_caption_track(&tracks).is_none());
    }

    #[test]
    fn playable_ok_is_accepted() {
        let player = PlayerResponse {
            playability_status: Some(PlayabilityStatus {
                status: Some("OK".to_string()),
                reason: None,
            }),
            video_details: None,
            captions: None,
        };

        ensure_playable(&player).unwrap();
    }

    #[test]
    fn non_ok_playability_fails() {
        let player = PlayerResponse {
            playability_status: Some(PlayabilityStatus {
                status: Some("ERROR".to_string()),
                reason: Some("private video".to_string()),
            }),
            video_details: None,
            captions: None,
        };

        assert!(matches!(
            ensure_playable(&player),
            Err(YoutubeError::Player(_))
        ));
    }
}
