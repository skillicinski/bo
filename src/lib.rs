use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::process;
use std::rc::Rc;
use std::time::{SystemTime, UNIX_EPOCH};

use htmd::{HtmlToMarkdown, Node};
use markup5ever_rcdom::NodeData;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use url::Url;

mod agent;

pub fn run_agent(args: impl Iterator<Item = String>) -> Result<(), String> {
    agent::run(args)
}

const STATE_FILE: &str = "state.json";

const ADJECTIVES: &[&str] = &[
    "amber", "brisk", "calm", "clever", "crisp", "eager", "gentle", "hidden", "mellow", "quiet",
    "rapid", "silver", "steady", "tidy", "vivid",
];
const NOUNS: &[&str] = &[
    "badger", "cedar", "comet", "falcon", "meadow", "otter", "panda", "quartz", "river", "sparrow",
    "sunset", "thicket", "willow", "wren", "zephyr",
];

pub mod application {
    use std::fmt;

    #[derive(Debug, PartialEq, Eq)]
    pub enum SnapError {
        Input(String),
        Request(String),
        Http {
            status: u16,
            request_id: Option<String>,
        },
        Content(String),
        Filesystem(String),
        Unsupported(String),
    }

    impl fmt::Display for SnapError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::Input(detail) => write!(formatter, "input: {detail}"),
                Self::Request(detail) => write!(formatter, "request: {detail}"),
                Self::Http {
                    status,
                    request_id: Some(request_id),
                } => write!(formatter, "http: HTTP {status} (request_id: {request_id})"),
                Self::Http {
                    status,
                    request_id: None,
                } => write!(formatter, "http: HTTP {status}"),
                Self::Content(detail) => write!(formatter, "content: {detail}"),
                Self::Filesystem(detail) => write!(formatter, "filesystem: {detail}"),
                Self::Unsupported(detail) => write!(formatter, "unsupported: {detail}"),
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    pub struct SnapCommandError {
        pub completed: Vec<SnapOutcome>,
        pub source_url: Option<String>,
        pub error: SnapError,
    }

    impl SnapCommandError {
        pub fn input(detail: impl Into<String>) -> Self {
            Self {
                completed: Vec::new(),
                source_url: None,
                error: SnapError::Input(detail.into()),
            }
        }
    }

    impl fmt::Display for SnapCommandError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            if let Some(source_url) = &self.source_url {
                write!(formatter, "{source_url} ({})", self.error)
            } else {
                self.error.fmt(formatter)
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    pub struct SnapOutcome {
        pub source_url: String,
        pub result: Result<String, SnapError>,
    }

    pub struct SnapReport {
        pub outcomes: Vec<SnapOutcome>,
    }

    pub fn seed(requested_name: Option<&str>) -> Result<std::path::PathBuf, String> {
        let home = super::home_dir()?;
        super::seed_at(&home, requested_name)
    }

    pub fn state(name: &str, full: bool) -> Result<String, String> {
        super::validate_name(name)?;
        let home = super::home_dir()?;
        let target = home.join(".bo").join(name);
        if !target.is_dir() {
            return Err(format!(
                "target directory does not exist: {}",
                target.display()
            ));
        }
        let state = super::load_state(&target)?;
        if full {
            serde_json::to_string_pretty(&state).map_err(|error| error.to_string())
        } else {
            Ok(format!("{} documents snapped", state.raw.len()))
        }
    }

    pub fn snap(name: &str, urls: &[String]) -> Result<SnapReport, SnapCommandError> {
        super::validate_name(name).map_err(|error| SnapCommandError {
            completed: Vec::new(),
            source_url: None,
            error: SnapError::Input(error),
        })?;
        let home = super::home_dir().map_err(|error| SnapCommandError {
            completed: Vec::new(),
            source_url: None,
            error: SnapError::Filesystem(error),
        })?;
        let target = home.join(".bo").join(name);
        if !target.is_dir() {
            return Err(SnapCommandError {
                completed: Vec::new(),
                source_url: None,
                error: SnapError::Input(format!(
                    "target directory does not exist: {} (run bo seed --name {})",
                    target.display(),
                    name
                )),
            });
        }
        super::snap_at(&target, urls)
    }
}

pub(crate) fn home_dir() -> Result<PathBuf, String> {
    env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .or_else(|| env::var_os("USERPROFILE").filter(|value| !value.is_empty()))
        .map(PathBuf::from)
        .ok_or_else(|| "HOME or USERPROFILE is not set".to_string())
}

fn snap_at(
    target: &Path,
    urls: &[String],
) -> Result<application::SnapReport, application::SnapCommandError> {
    let mut state = load_state(target).map_err(|error| application::SnapCommandError {
        completed: Vec::new(),
        source_url: None,
        error: application::SnapError::Filesystem(error),
    })?;
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("bo/0.1")
        .build()
        .map_err(|error| application::SnapCommandError {
            completed: Vec::new(),
            source_url: None,
            error: application::SnapError::Request(error.to_string()),
        })?;
    let mut outcomes = Vec::new();

    for url in urls {
        match snap_one(&client, target, url) {
            Ok(snapshot) => {
                state.raw.push(RawRecord {
                    filename: snapshot.filename.clone(),
                    url: url.clone(),
                    written_at: snapshot.written_at,
                });
                if let Err(error) = write_state(target, &state) {
                    return Err(application::SnapCommandError {
                        completed: outcomes,
                        source_url: Some(url.clone()),
                        error: application::SnapError::Filesystem(format!(
                            "updating state failed: {error}"
                        )),
                    });
                }
                outcomes.push(application::SnapOutcome {
                    source_url: url.clone(),
                    result: Ok(snapshot.filename),
                });
            }
            Err(error) => {
                outcomes.push(application::SnapOutcome {
                    source_url: url.clone(),
                    result: Err(error),
                });
            }
        }
    }

    Ok(application::SnapReport { outcomes })
}

fn snap_one(
    client: &Client,
    target: &Path,
    input: &str,
) -> Result<Snapshot, application::SnapError> {
    let url = Url::parse(input)
        .map_err(|error| application::SnapError::Input(format!("invalid URL: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(application::SnapError::Input(
            "URL scheme must be http or https".into(),
        ));
    }
    if url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("youtube.com")
            || host.ends_with(".youtube.com")
            || host.eq_ignore_ascii_case("youtu.be")
    }) {
        return Err(application::SnapError::Unsupported(
            "YouTube transcription is not supported".into(),
        ));
    }

    let response = client
        .get(url)
        .send()
        .map_err(|error| application::SnapError::Request(format!("request failed: {error}")))?;
    if !response.status().is_success() {
        let request_id = response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        return Err(application::SnapError::Http {
            status: response.status().as_u16(),
            request_id,
        });
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !is_html_content_type(content_type) {
        return Err(application::SnapError::Content(format!(
            "not HTML (Content-Type: {content_type})"
        )));
    }

    let html = response.text().map_err(|error| {
        application::SnapError::Content(format!("reading response failed: {error}"))
    })?;
    let page = extract_page(&html).map_err(application::SnapError::Content)?;
    write_snapshot(target, &page.title, &page.markdown)
}

fn is_html_content_type(content_type: &str) -> bool {
    matches!(
        content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "text/html" | "application/xhtml+xml"
    )
}

struct Page {
    title: String,
    markdown: String,
}

struct Snapshot {
    filename: String,
    written_at: u128,
}

#[derive(Default, Deserialize, Serialize)]
pub(crate) struct State {
    pub(crate) raw: Vec<RawRecord>,
    pub(crate) summaries: Vec<SummaryRecord>,
}

#[derive(Deserialize, Serialize)]
pub(crate) struct RawRecord {
    pub(crate) filename: String,
    pub(crate) url: String,
    pub(crate) written_at: u128,
}

#[derive(Deserialize, Serialize, Clone)]
pub(crate) struct SummaryRecord {
    pub(crate) filename: String,
    pub(crate) source_key: String,
    pub(crate) derived_from: String,
    pub(crate) created_at: u128,
    pub(crate) updated_at: u128,
}

fn extract_page(html: &str) -> Result<Page, String> {
    let converter = HtmlToMarkdown::builder()
        .skip_tags(vec![
            "aside", "footer", "form", "header", "nav", "noscript", "script", "style", "svg",
            "template",
        ])
        .build();
    let document = converter
        .html_to_tree(html)
        .map_err(|error| format!("HTML parsing failed: {error}"))?;
    let title = find_element(&document, "title")
        .map(|element| text_content(&element))
        .map(|title| title.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    if title.is_empty() {
        return Err("page has no title".to_string());
    }

    for tag in ["article", "main", "body"] {
        if let Some(element) = find_element(&document, tag) {
            let markdown = converter.tree_to_markdown(&element);
            let markdown = remove_matching_title_heading(&markdown, &title);
            if markdown.chars().any(char::is_alphanumeric) {
                return Ok(Page {
                    title: title.clone(),
                    markdown: format!("# {title}\n\n{}\n", markdown.trim()),
                });
            }
        }
    }

    Err("page has no readable content".to_string())
}

fn remove_matching_title_heading(markdown: &str, title: &str) -> String {
    let mut lines = markdown.lines();
    let Some(first) = lines.next() else {
        return String::new();
    };
    if first
        .strip_prefix("# ")
        .is_some_and(|heading| heading.trim().eq_ignore_ascii_case(title.trim()))
    {
        lines.collect::<Vec<_>>().join("\n").trim().to_string()
    } else {
        markdown.trim().to_string()
    }
}

fn find_element(node: &Rc<Node>, wanted_tag: &str) -> Option<Rc<Node>> {
    if matches!(&node.data, NodeData::Element { name, .. } if name.local.as_ref() == wanted_tag) {
        return Some(Rc::clone(node));
    }
    node.children
        .borrow()
        .iter()
        .find_map(|child| find_element(child, wanted_tag))
}

fn text_content(node: &Rc<Node>) -> String {
    let mut text = match &node.data {
        NodeData::Text { contents } => contents.borrow().to_string(),
        _ => String::new(),
    };
    for child in node.children.borrow().iter() {
        text.push_str(&text_content(child));
    }
    text
}

fn write_snapshot(
    target: &Path,
    title: &str,
    markdown: &str,
) -> Result<Snapshot, application::SnapError> {
    let slug = kebab_case(title)
        .ok_or_else(|| application::SnapError::Content("title cannot produce a filename".into()))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| application::SnapError::Filesystem(error.to_string()))?
        .as_millis();

    for attempt in 0.. {
        let filename = match attempt {
            0 => format!("{slug}.md"),
            1 => format!("{slug}--{timestamp}.md"),
            number => format!("{slug}--{timestamp}--{number}.md"),
        };
        let path = target.join(&filename);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(markdown.as_bytes()).map_err(|error| {
                    application::SnapError::Filesystem(format!(
                        "writing {} failed: {error}",
                        path.display()
                    ))
                })?;
                return Ok(Snapshot {
                    filename,
                    written_at: timestamp,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(application::SnapError::Filesystem(format!(
                    "creating {} failed: {error}",
                    path.display()
                )))
            }
        }
    }

    unreachable!()
}

fn kebab_case(value: &str) -> Option<String> {
    let mut slug = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            slug.extend(character.to_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    (!slug.is_empty()).then_some(slug)
}

fn seed_at(home: &Path, requested_name: Option<&str>) -> Result<PathBuf, String> {
    let name = match requested_name {
        Some(name) => name.to_owned(),
        None => random_name().map_err(|error| error.to_string())?,
    };
    validate_name(&name)?;

    let root = home.join(".bo");
    fs::create_dir_all(&root).map_err(|error| error.to_string())?;
    let path = root.join(&name);
    fs::create_dir(&path).map_err(|error| error.to_string())?;
    write_state(&path, &State::default())?;
    Ok(path)
}

pub(crate) fn load_state(target: &Path) -> Result<State, String> {
    let path = target.join(STATE_FILE);
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("reading {} failed: {error}", path.display()))?;
    serde_json::from_str(&contents)
        .map_err(|error| format!("parsing {} failed: {error}", path.display()))
}

pub(crate) fn write_state(target: &Path, state: &State) -> Result<(), String> {
    let path = target.join(STATE_FILE);
    let temporary_path = target.join(format!(".{STATE_FILE}.tmp"));
    let contents = serde_json::to_string_pretty(state)
        .map_err(|error| format!("serializing {} failed: {error}", path.display()))?;
    let write_result = (|| -> io::Result<()> {
        let mut file = File::create(&temporary_path)?;
        file.write_all(format!("{contents}\n").as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary_path, &path)
    })();
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("writing {} failed: {error}", path.display()));
    }
    Ok(())
}

pub(crate) fn validate_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("name must not be empty".to_string());
    }
    if name == "." || name == ".." {
        return Err("name must be a single directory component".to_string());
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Err("name must not end with a dot or space".to_string());
    }
    if name.chars().any(|character| {
        character.is_control()
            || matches!(
                character,
                '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
            )
    }) {
        return Err("name contains an invalid character".to_string());
    }
    if is_reserved_device_name(name) {
        return Err("name is reserved on Windows".to_string());
    }
    Ok(())
}

fn is_reserved_device_name(name: &str) -> bool {
    let stem = name.split('.').next().unwrap_or_default();
    let bytes = stem.as_bytes();
    stem.eq_ignore_ascii_case("CON")
        || stem.eq_ignore_ascii_case("PRN")
        || stem.eq_ignore_ascii_case("AUX")
        || stem.eq_ignore_ascii_case("NUL")
        || (bytes.len() == 4
            && (bytes[..3].eq_ignore_ascii_case(b"COM") || bytes[..3].eq_ignore_ascii_case(b"LPT"))
            && bytes[3].is_ascii_digit()
            && bytes[3] != b'0')
}

fn random_name() -> io::Result<String> {
    let mut bytes = [0; 2];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(format!(
        "{}-{}",
        ADJECTIVES[usize::from(bytes[0]) % ADJECTIVES.len()],
        NOUNS[usize::from(bytes[1]) % NOUNS.len()]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[track_caller]
    fn check_name(name: &str, expected_valid: bool) {
        assert_eq!(
            validate_name(name).is_ok(),
            expected_valid,
            "name: {name:?}"
        );
    }

    #[test]
    fn generated_names_are_lowercase_adjective_noun_names() {
        let name = random_name().unwrap();

        assert!(name
            .chars()
            .all(|character| character.is_ascii_lowercase() || character == '-'));
        assert_eq!(name.matches('-').count(), 1);
        assert!(validate_name(&name).is_ok());
    }

    #[test]
    fn names_are_validated_as_portable_components() {
        for name in [
            "",
            ".",
            "..",
            "../escape",
            "sub/name",
            "sub\\name",
            "CON",
            "CON.txt",
            "COM1",
            "LPT9",
            "a:b",
            "a*b",
            "a?b",
            "a\"b",
            "a<b",
            "a>b",
            "a|b",
            "name.",
            "name ",
            "line\nbreak",
        ] {
            check_name(name, false);
        }

        for name in ["test", "hello-world", "has space", ".hidden", "café"] {
            check_name(name, true);
        }
    }

    #[test]
    fn state_write_failure_preserves_previous_state() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let target = env::temp_dir().join(format!("bo-state-write-{}-{suffix}", process::id()));
        fs::create_dir(&target).unwrap();
        fs::write(target.join(STATE_FILE), "previous state\n").unwrap();
        fs::create_dir(target.join(format!(".{STATE_FILE}.tmp"))).unwrap();

        assert!(write_state(&target, &State::default()).is_err());
        assert_eq!(
            fs::read_to_string(target.join(STATE_FILE)).unwrap(),
            "previous state\n"
        );

        fs::remove_dir_all(target).unwrap();
    }
}
