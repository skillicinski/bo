use url::Url;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupportedYoutubeUrl {
    pub video_id: String,
    pub normalized_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum YoutubeUrlMatch {
    NotYoutube,
    Invalid { message: String },
    Supported(SupportedYoutubeUrl),
    Unsupported { url: String, reason: String },
}

pub fn classify_url(input: &str) -> YoutubeUrlMatch {
    let parsed = match Url::parse(input) {
        Ok(parsed) => parsed,
        Err(_) => return YoutubeUrlMatch::NotYoutube,
    };

    if !matches!(parsed.scheme(), "http" | "https") {
        if is_youtube_like_host(parsed.host_str()) {
            return YoutubeUrlMatch::Unsupported {
                url: parsed.as_str().to_string(),
                reason: format!("scheme '{}' is not supported", parsed.scheme()),
            };
        }
        return YoutubeUrlMatch::NotYoutube;
    }

    let Some(host) = parsed.host_str().map(|h| h.to_ascii_lowercase()) else {
        return YoutubeUrlMatch::NotYoutube;
    };

    if host == "youtu.be" {
        return classify_youtu_be(parsed);
    }

    if host == "youtube.com" || host == "www.youtube.com" {
        return classify_youtube_host(parsed);
    }

    if is_youtube_like_host(Some(&host)) {
        return YoutubeUrlMatch::Unsupported {
            url: parsed.as_str().to_string(),
            reason: format!("host '{}' is not supported", host),
        };
    }

    YoutubeUrlMatch::NotYoutube
}

fn classify_youtu_be(parsed: Url) -> YoutubeUrlMatch {
    let normalized_url = parsed.as_str().to_string();
    let path = parsed.path().trim_matches('/');
    match validate_video_id(path) {
        Ok(video_id) => YoutubeUrlMatch::Supported(SupportedYoutubeUrl {
            video_id,
            normalized_url,
        }),
        Err(reason) => YoutubeUrlMatch::Unsupported {
            url: normalized_url,
            reason,
        },
    }
}

fn classify_youtube_host(parsed: Url) -> YoutubeUrlMatch {
    let normalized_url = parsed.as_str().to_string();
    match parsed.path() {
        "/watch" => {
            let video_id = parsed.query_pairs().find_map(|(key, value)| {
                if key == "v" {
                    Some(value.into_owned())
                } else {
                    None
                }
            });
            match video_id {
                Some(video_id) => match validate_video_id(&video_id) {
                    Ok(video_id) => YoutubeUrlMatch::Supported(SupportedYoutubeUrl {
                        video_id,
                        normalized_url,
                    }),
                    Err(reason) => YoutubeUrlMatch::Unsupported {
                        url: normalized_url,
                        reason,
                    },
                },
                None => YoutubeUrlMatch::Unsupported {
                    url: normalized_url,
                    reason: "watch URL is missing v parameter".to_string(),
                },
            }
        }
        path if path.starts_with("/shorts/") => {
            let id = path.trim_start_matches("/shorts/").trim_matches('/');
            match validate_video_id(id) {
                Ok(video_id) => YoutubeUrlMatch::Supported(SupportedYoutubeUrl {
                    video_id,
                    normalized_url,
                }),
                Err(reason) => YoutubeUrlMatch::Unsupported {
                    url: normalized_url,
                    reason,
                },
            }
        }
        path if path.starts_with("/embed/") => YoutubeUrlMatch::Unsupported {
            url: normalized_url,
            reason:
                "embed URLs are not collected; collect the containing page or original video URL"
                    .to_string(),
        },
        path if path.starts_with("/playlist") => YoutubeUrlMatch::Unsupported {
            url: normalized_url,
            reason: "playlist collection is out of scope".to_string(),
        },
        path if path.starts_with("/channel/") || path.starts_with("/@") => {
            YoutubeUrlMatch::Unsupported {
                url: normalized_url,
                reason: "channel collection is out of scope".to_string(),
            }
        }
        path if path.starts_with("/results") => YoutubeUrlMatch::Unsupported {
            url: normalized_url,
            reason: "search result collection is out of scope".to_string(),
        },
        _ => YoutubeUrlMatch::Unsupported {
            url: normalized_url,
            reason: "not a supported YouTube video URL".to_string(),
        },
    }
}

fn is_youtube_like_host(host: Option<&str>) -> bool {
    let Some(host) = host.map(|h| h.to_ascii_lowercase()) else {
        return false;
    };
    matches!(
        host.as_str(),
        "youtube.com"
            | "www.youtube.com"
            | "m.youtube.com"
            | "music.youtube.com"
            | "youtu.be"
            | "www.youtu.be"
            | "youtube-nocookie.com"
            | "www.youtube-nocookie.com"
    )
}

fn validate_video_id(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("video ID is missing".to_string());
    }
    if !value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err("video ID contains unsupported characters".to_string());
    }
    if value.len() != 11 {
        return Err("video ID must be 11 URL-safe characters".to_string());
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
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
        assert_eq!(result.video_id, "a1mhk7mAetk");
        assert_eq!(
            result.normalized_url,
            "https://www.youtube.com/watch?v=a1mhk7mAetk"
        );

        let result = supported("https://youtube.com/watch?v=1GXrAY-wzfk");
        assert_eq!(result.video_id, "1GXrAY-wzfk");
    }

    #[test]
    fn supports_watch_urls_with_extra_query_params() {
        let result = supported("https://www.youtube.com/watch?si=x&v=a1mhk7mAetk&t=30s&list=abc");
        assert_eq!(result.video_id, "a1mhk7mAetk");
        assert_eq!(
            result.normalized_url,
            "https://www.youtube.com/watch?si=x&v=a1mhk7mAetk&t=30s&list=abc"
        );
    }

    #[test]
    fn supports_short_urls_without_canonicalizing() {
        let result = supported("https://youtu.be/a1mhk7mAetk?si=x&t=30");
        assert_eq!(result.video_id, "a1mhk7mAetk");
        assert_eq!(
            result.normalized_url,
            "https://youtu.be/a1mhk7mAetk?si=x&t=30"
        );
    }

    #[test]
    fn supports_shorts_urls() {
        let result = supported("https://www.youtube.com/shorts/a1mhk7mAetk?feature=share");
        assert_eq!(result.video_id, "a1mhk7mAetk");
    }

    #[test]
    fn host_matching_is_case_insensitive() {
        let result = supported("https://WWW.YouTube.COM/watch?v=a1mhk7mAetk");
        assert_eq!(result.video_id, "a1mhk7mAetk");
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
        assert!(
            unsupported("https://www.youtube.com/results?search_query=rust").contains("search")
        );
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
}
