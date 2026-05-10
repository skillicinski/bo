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
