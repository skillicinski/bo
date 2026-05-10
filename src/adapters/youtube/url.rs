use url::Url;

#[derive(Clone, Debug, Eq, PartialEq)]
struct VideoId(String);

impl VideoId {
    fn parse(value: &str) -> Result<Self, String> {
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
        Ok(Self(value.to_string()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupportedYoutubeUrl {
    video_id: VideoId,
    normalized_url: String,
}

impl SupportedYoutubeUrl {
    pub fn video_id(&self) -> &str {
        self.video_id.as_str()
    }

    pub fn normalized_url(&self) -> &str {
        &self.normalized_url
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum YoutubeUrlMatch {
    NotYoutube,
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
    match VideoId::parse(path) {
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
                Some(video_id) => match VideoId::parse(&video_id) {
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
            match VideoId::parse(id) {
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

#[cfg(test)]
#[path = "../../tests/adapters_youtube_url_tests.rs"]
mod tests;
