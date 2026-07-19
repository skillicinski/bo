// bo show — deterministic inspection for a single collected leaf.

use crate::cli::json::JsonError;
use crate::cli::resolve_leaf_path;
use crate::domain::state::{self};
use crate::domain::tree::TreeLoadState;
use crate::domain::{Leaf, Title, Url};
use crate::engine::config::SeededConfig;
use serde::Serialize;
use serde_json::json;
use serde_yaml_ng::{Mapping, Value};
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::Path;

// ── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
pub struct ShowCandidateSummary {
    pub file: String,
    pub title: String,
    pub path: String,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShowResult {
    pub title: String,
    pub file: String,
    pub path: String,
    pub url: Option<String>,
    pub frontmatter: Mapping,
    pub frontmatter_raw: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub full: bool,
}

#[derive(Debug)]
pub enum ShowError {
    Io(io::Error),
    TreeState(state::TreeStateError),
    NotFound {
        title: String,
    },
    Ambiguous {
        title: String,
        candidates: Vec<ShowCandidateSummary>,
    },
    SuspiciousPath {
        file: String,
    },
    MissingFile {
        file: String,
    },
    UnreadableFile {
        file: String,
        source: io::Error,
    },
    InvalidFrontmatter {
        file: String,
        reason: String,
    },
}

impl fmt::Display for ShowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ShowError::Io(e) => write!(f, "I/O error: {}", e),
            ShowError::TreeState(e) => write!(f, "{}", e),
            ShowError::NotFound { title } => write!(
                f,
                "leaf title '{title}' not found; run `bo list` to inspect available leaves"
            ),
            ShowError::Ambiguous { title, candidates } => {
                write!(f, "leaf title '{title}' is ambiguous; matches:")?;
                for candidate in candidates {
                    write!(f, "\n- {} ({})", candidate.title, candidate.file)?;
                    if !candidate.path.is_empty() {
                        write!(f, " at {}", candidate.path)?;
                    }
                    if let Some(url) = &candidate.url {
                        write!(f, " — {}", url)?;
                    }
                }
                Ok(())
            }
            ShowError::SuspiciousPath { file } => {
                write!(f, "cannot show '{file}': suspicious path")
            }
            ShowError::MissingFile { file } => write!(
                f,
                "cannot show '{file}': file is missing \u{2014} the file was deleted or moved.\nrun `bo synthesize` to clean up the stale state record."
            ),
            ShowError::UnreadableFile { file, source } => {
                write!(f, "cannot show '{file}': unreadable file: {source}")
            }
            ShowError::InvalidFrontmatter { file, reason } => {
                write!(f, "cannot show '{file}': invalid frontmatter: {reason}")
            }
        }
    }
}

impl From<io::Error> for ShowError {
    fn from(e: io::Error) -> Self {
        ShowError::Io(e)
    }
}

impl ShowError {
    pub fn json_error(&self) -> JsonError {
        match self {
            ShowError::NotFound { title } => {
                JsonError::with_details("not_found", self.to_string(), json!({ "title": title }))
            }
            ShowError::Ambiguous { title, candidates } => JsonError::with_details(
                "ambiguous",
                self.to_string(),
                json!({ "title": title, "candidates": candidates }),
            ),
            ShowError::Io(_) => JsonError::new("io_error", self.to_string()),
            ShowError::TreeState(_) => JsonError::new("state_error", self.to_string()),
            ShowError::SuspiciousPath { file } => JsonError::with_details(
                "suspicious_path",
                self.to_string(),
                json!({ "file": file }),
            ),
            ShowError::MissingFile { file } => {
                JsonError::with_details("not_found", self.to_string(), json!({ "file": file }))
            }
            ShowError::InvalidFrontmatter { file, .. } => JsonError::with_details(
                "validation_error",
                self.to_string(),
                json!({ "file": file }),
            ),
            ShowError::UnreadableFile { file, source: _ } => {
                JsonError::with_details("io_error", self.to_string(), json!({ "file": file }))
            }
        }
    }
}

pub fn run(cfg: &SeededConfig, title: &str, full: bool) -> Result<ShowResult, ShowError> {
    let tree = cfg.tree();
    show_leaf(tree.path(), title, full)
}

// ── show ─────────────────────────────────────────────────────────────────────

pub(crate) fn show_leaf(tree_dir: &Path, title: &str, full: bool) -> Result<ShowResult, ShowError> {
    let requested_title = normalize_title(title);
    if title.is_empty() {
        return Err(ShowError::NotFound {
            title: title.to_string(),
        });
    }

    let state = match crate::engine::state::load_state(tree_dir) {
        Ok(TreeLoadState::Loaded(state)) => state,
        Ok(TreeLoadState::FreshSeeded) => {
            return Err(ShowError::NotFound {
                title: title.to_string(),
            });
        }
        Ok(TreeLoadState::MissingState) => {
            return Err(ShowError::TreeState(
                state::TreeStateError::TreeNotInitialized,
            ));
        }
        Err(e) => return Err(ShowError::TreeState(e)),
    };
    let canonical_tree_dir = fs::canonicalize(tree_dir).ok();

    let mut matches = Vec::new();
    for leaf in &state.leaves {
        let candidate = load_candidate(tree_dir, canonical_tree_dir.as_deref(), leaf);
        let title_match = match &candidate {
            CandidateLoad::Loaded(loaded) => {
                normalize_title(&loaded.summary.title) == requested_title
            }
            CandidateLoad::Broken { summary, .. } => {
                normalize_title(&summary.title) == requested_title
            }
        };
        if title_match {
            matches.push(candidate);
        }
    }

    match matches.len() {
        0 => Err(ShowError::NotFound {
            title: title.to_string(),
        }),
        1 => match matches.remove(0) {
            CandidateLoad::Loaded(leaf) => Ok(build_result(leaf, full)),
            CandidateLoad::Broken { error, .. } => Err(error),
        },
        _ => Err(ShowError::Ambiguous {
            title: title.to_string(),
            candidates: matches.into_iter().map(CandidateLoad::summary).collect(),
        }),
    }
}

fn load_candidate(
    tree_dir: &Path,
    canonical_tree_dir: Option<&Path>,
    leaf: &Leaf,
) -> CandidateLoad {
    let fallback_title = leaf
        .title
        .as_ref()
        .map(|t| t.as_str().to_string())
        .unwrap_or_default();
    let fallback_url = Some(leaf.url.as_str().to_string());
    let unresolved_summary = ShowCandidateSummary {
        file: leaf.file.clone(),
        title: fallback_title.clone(),
        path: leaf.file.clone(),
        url: fallback_url.clone(),
    };

    let path = match resolve_leaf_path(tree_dir, canonical_tree_dir, &leaf.file) {
        Ok(path) => path,
        Err(_) => {
            return CandidateLoad::Broken {
                summary: unresolved_summary,
                error: ShowError::SuspiciousPath {
                    file: leaf.file.clone(),
                },
            };
        }
    };

    let path_string = path.display().to_string();
    let fallback_summary = ShowCandidateSummary {
        path: path_string.clone(),
        ..unresolved_summary
    };

    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return CandidateLoad::Broken {
                summary: fallback_summary,
                error: ShowError::MissingFile {
                    file: leaf.file.clone(),
                },
            };
        }
        Err(e) => {
            return CandidateLoad::Broken {
                summary: fallback_summary,
                error: ShowError::UnreadableFile {
                    file: leaf.file.clone(),
                    source: e,
                },
            };
        }
    };

    let document = match parse_leaf_document(&content) {
        Ok(document) => document,
        Err(reason) => {
            return CandidateLoad::Broken {
                summary: fallback_summary,
                error: ShowError::InvalidFrontmatter {
                    file: leaf.file.clone(),
                    reason,
                },
            };
        }
    };

    let typed_fm = ShowFrontmatter::from_mapping(&document.frontmatter);
    let title = typed_fm
        .title
        .as_ref()
        .map(|t| t.as_str().to_string())
        .or_else(|| leaf.title.as_ref().map(|t| t.as_str().to_string()))
        .unwrap_or_default();
    let url = typed_fm
        .url
        .as_ref()
        .map(|u| u.as_str().to_string())
        .or(fallback_url);

    CandidateLoad::Loaded(LoadedLeaf {
        summary: ShowCandidateSummary {
            file: leaf.file.clone(),
            title,
            path: path_string,
            url,
        },
        frontmatter: document.frontmatter,
        frontmatter_raw: document.frontmatter_raw,
        body: document.body,
    })
}

fn build_result(leaf: LoadedLeaf, full: bool) -> ShowResult {
    ShowResult {
        title: leaf.summary.title,
        file: leaf.summary.file,
        path: leaf.summary.path,
        url: leaf.summary.url,
        frontmatter: leaf.frontmatter,
        frontmatter_raw: leaf.frontmatter_raw,
        body: full.then_some(leaf.body),
        full,
    }
}

fn parse_leaf_document(content: &str) -> Result<LeafDocument, String> {
    // Guard: no opening delimiter means no frontmatter at all.
    if !content.starts_with("---\n") {
        return Ok(LeafDocument {
            frontmatter: Mapping::new(),
            frontmatter_raw: String::new(),
            body: content.to_string(),
        });
    }

    // Search for closing delimiter after the opening "---\n".
    let after_open = &content["---\n".len()..];
    let (close_delim, close_pos) = after_open
        .find("\n---\n")
        .map(|pos| ("\n---\n", pos))
        .or_else(|| {
            after_open
                .ends_with("\n---")
                .then(|| ("\n---", after_open.len() - "\n---".len()))
        })
        .or_else(|| after_open.starts_with("---\n").then_some(("---\n", 0)))
        .or_else(|| (after_open == "---").then_some(("---", 0)))
        .ok_or_else(|| "no frontmatter delimiters found".to_string())?;

    // YAML text between opening and closing delimiters.
    let fm_text = &after_open[..close_pos];
    let frontmatter = serde_yaml_ng::from_str::<Mapping>(fm_text).map_err(|e| e.to_string())?;

    // raw_end: span from content[0] to just past the closing delimiter.
    // The delimiter itself includes the trailing newline, so no extra \n check needed.
    let raw_end = "---\n".len() + close_pos + close_delim.len();

    // Body: everything after the closing delimiter.
    let mut body = content[raw_end..].to_string();
    // Strip one \n (newline right after delimiter), then one blank-line separator \n.
    if body.starts_with('\n') {
        body = body[1..].to_string();
    }
    if body.starts_with('\n') {
        body = body[1..].to_string();
    }

    Ok(LeafDocument {
        frontmatter,
        frontmatter_raw: content[..raw_end].to_string(),
        body,
    })
}

fn normalize_title(title: &str) -> String {
    title.to_lowercase()
}

// ── render ───────────────────────────────────────────────────────────────────

pub fn render_human(result: &ShowResult) -> String {
    let mut output = String::new();
    output.push_str(&result.frontmatter_raw);
    if !output.ends_with('\n') {
        output.push('\n');
    }

    if let Some(body) = &result.body {
        output.push('\n');
        output.push_str(body);
        if !body.ends_with('\n') {
            output.push('\n');
        }
    }

    output
}

// ── internal types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct LeafDocument {
    frontmatter: Mapping,
    frontmatter_raw: String,
    body: String,
}

/// Read-side typed frontmatter — the fields show consumes. Extracted from the
/// YAML Mapping with per-field tolerance: mistyped scalars (e.g. `title: 123`)
/// degrade to None instead of erroring, keeping show's read/inspect contract.
/// The full mapping (including unknown keys) flows to the JSON envelope untouched.
#[derive(Debug, Clone)]
struct ShowFrontmatter {
    title: Option<Title>,
    url: Option<Url>,
}

impl ShowFrontmatter {
    fn from_mapping(fm: &Mapping) -> Self {
        Self {
            title: fm
                .get("title")
                .and_then(Value::as_str)
                .and_then(|s| Title::parse(s).ok()),
            url: fm
                .get("url")
                .and_then(Value::as_str)
                .and_then(|s| Url::parse(s).ok()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct LoadedLeaf {
    summary: ShowCandidateSummary,
    frontmatter: Mapping,
    frontmatter_raw: String,
    body: String,
}

#[derive(Debug)]
enum CandidateLoad {
    Loaded(LoadedLeaf),
    Broken {
        summary: ShowCandidateSummary,
        error: ShowError,
    },
}

impl CandidateLoad {
    fn summary(self) -> ShowCandidateSummary {
        match self {
            CandidateLoad::Loaded(leaf) => leaf.summary,
            CandidateLoad::Broken { summary, .. } => summary,
        }
    }
}

#[cfg(test)]
#[path = "../tests/cli_show_tests.rs"]
mod tests;
