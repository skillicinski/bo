use super::*;

#[test]
#[ignore]
fn fetches_known_captioned_watch_video() {
    assert_captioned_video_collects("https://www.youtube.com/watch?v=a1mhk7mAetk");
}

#[test]
#[ignore]
fn fetches_known_captioned_youtu_be_video() {
    assert_captioned_video_collects("https://youtu.be/a1mhk7mAetk");
}

#[test]
#[ignore]
fn fetches_known_captioned_shorts_url() {
    assert_captioned_video_collects("https://www.youtube.com/shorts/a1mhk7mAetk");
}

fn assert_captioned_video_collects(url: &str) {
    let doc = collect_transcript(url).unwrap();
    assert!(!doc.title.trim().is_empty());
    assert!(!doc.body_markdown.trim().is_empty());
    assert!(!doc.body_markdown.contains("ytInitialData"));
    assert!(!doc.body_markdown.contains("<html"));
    assert!(!doc.body_markdown.contains("<script"));
    assert!(!doc.body_markdown.contains("Sign in to confirm"));
    assert!(!doc
        .body_markdown
        .lines()
        .any(|line| line.trim_start().starts_with("0:00 ")));
}
