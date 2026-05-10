use super::*;

fn supported(input: &str) -> SupportedYoutubeUrl {
    match classify_url(input) {
        YoutubeUrlMatch::Supported(supported) => supported,
        other => panic!("expected supported URL, got {other:?}"),
    }
}

fn unsupported(input: &str) -> String {
    match classify_url(input) {
        YoutubeUrlMatch::Unsupported { reason, .. } => reason,
        other => panic!("expected unsupported URL, got {other:?}"),
    }
}

#[test]
fn supports_watch_urls() {
    let result = supported("https://www.youtube.com/watch?v=a1mhk7mAetk");
    assert_eq!(result.video_id(), "a1mhk7mAetk");
    assert_eq!(
        result.normalized_url(),
        "https://www.youtube.com/watch?v=a1mhk7mAetk"
    );

    let result = supported("https://youtube.com/watch?v=1GXrAY-wzfk");
    assert_eq!(result.video_id(), "1GXrAY-wzfk");
}

#[test]
fn supports_watch_urls_with_extra_query_params() {
    let result = supported("https://www.youtube.com/watch?si=x&v=a1mhk7mAetk&t=30s&list=abc");
    assert_eq!(result.video_id(), "a1mhk7mAetk");
    assert_eq!(
        result.normalized_url(),
        "https://www.youtube.com/watch?si=x&v=a1mhk7mAetk&t=30s&list=abc"
    );
}

#[test]
fn supports_short_urls_without_canonicalizing() {
    let result = supported("https://youtu.be/a1mhk7mAetk?si=x&t=30");
    assert_eq!(result.video_id(), "a1mhk7mAetk");
    assert_eq!(
        result.normalized_url(),
        "https://youtu.be/a1mhk7mAetk?si=x&t=30"
    );
}

#[test]
fn supports_shorts_urls() {
    let result = supported("https://www.youtube.com/shorts/a1mhk7mAetk?feature=share");
    assert_eq!(result.video_id(), "a1mhk7mAetk");
}

#[test]
fn host_matching_is_case_insensitive() {
    let result = supported("https://WWW.YouTube.COM/watch?v=a1mhk7mAetk");
    assert_eq!(result.video_id(), "a1mhk7mAetk");
}

#[test]
fn non_youtube_url_is_not_youtube() {
    assert_eq!(
        classify_url("https://example.com/watch?v=a1mhk7mAetk"),
        YoutubeUrlMatch::NotYoutube
    );
}

#[test]
fn malformed_url_is_not_youtube() {
    assert_eq!(classify_url("not a url"), YoutubeUrlMatch::NotYoutube);
}

#[test]
fn rejects_embed_urls() {
    assert!(unsupported("https://www.youtube.com/embed/a1mhk7mAetk").contains("embed"));
}

#[test]
fn rejects_non_video_youtube_urls() {
    assert!(unsupported("https://www.youtube.com/playlist?list=abc").contains("playlist"));
    assert!(unsupported("https://www.youtube.com/channel/abc").contains("channel"));
    assert!(unsupported("https://www.youtube.com/@somewhere").contains("channel"));
    assert!(unsupported("https://www.youtube.com/results?search_query=rust").contains("search"));
}

#[test]
fn rejects_missing_or_malformed_ids() {
    assert!(unsupported("https://www.youtube.com/watch").contains("missing"));
    assert!(unsupported("https://www.youtube.com/watch?v=").contains("missing"));
    assert!(unsupported("https://youtu.be/").contains("missing"));
    assert!(unsupported("https://youtu.be/short").contains("11"));
    assert!(unsupported("https://youtu.be/a1mhk7mAet!").contains("unsupported"));
    assert!(
        unsupported("https://www.youtube.com/shorts/a1mhk7mAetk/extra").contains("unsupported")
    );
}

#[test]
fn rejects_unsupported_youtube_like_hosts_and_schemes() {
    assert!(unsupported("https://m.youtube.com/watch?v=a1mhk7mAetk").contains("host"));
    assert!(unsupported("https://music.youtube.com/watch?v=a1mhk7mAetk").contains("host"));
    assert!(unsupported("https://www.youtube-nocookie.com/embed/a1mhk7mAetk").contains("host"));
    assert!(unsupported("ftp://www.youtube.com/watch?v=a1mhk7mAetk").contains("scheme"));
}
