// bo show — deterministic inspection for a single collected leaf.

use crate::domain::index;
use crate::domain::tree;
use serde::Serialize;
use serde_yaml_ng::{Mapping, Value};
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};

const PREVIEW_CHAR_LIMIT: usize = 2_000;

// ── public types ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Eq, PartialEq)]
pub struct ShowOptions {
    pub full: bool,
}

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
    pub body: String,
    pub truncated: bool,
    pub full: bool,
}

#[derive(Debug)]
pub enum ShowError {
    Io(io::Error),
    Json(serde_json::Error),
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
            ShowError::Json(e) => write!(f, "JSON error: {}", e),
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
            ShowError::MissingFile { file } => write!(f, "cannot show '{file}': missing file"),
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

impl From<serde_json::Error> for ShowError {
    fn from(e: serde_json::Error) -> Self {
        ShowError::Json(e)
    }
}

// ── show ─────────────────────────────────────────────────────────────────────

pub fn show_leaf(
    tree_dir: &Path,
    title: &str,
    options: &ShowOptions,
) -> Result<ShowResult, ShowError> {
    let requested_title = normalize_title(title);
    if title.is_empty() {
        return Err(ShowError::NotFound {
            title: title.to_string(),
        });
    }

    let index_path = tree::index_path(tree_dir);
    let entries = index::read_index(&index_path)?;
    let canonical_tree_dir = fs::canonicalize(tree_dir).ok();

    let mut matches = Vec::new();
    for entry in &entries {
        match load_candidate(tree_dir, canonical_tree_dir.as_deref(), entry) {
            CandidateLoad::Loaded(leaf) => {
                if normalize_title(&leaf.summary.title) == requested_title {
                    matches.push(MatchedCandidate::Loaded(leaf));
                }
            }
            CandidateLoad::Broken { summary, error } => {
                if normalize_title(&summary.title) == requested_title {
                    matches.push(MatchedCandidate::Broken { summary, error });
                }
            }
        }
    }

    match matches.len() {
        0 => Err(ShowError::NotFound {
            title: title.to_string(),
        }),
        1 => match matches.remove(0) {
            MatchedCandidate::Loaded(leaf) => Ok(build_result(leaf, options)),
            MatchedCandidate::Broken { error, .. } => Err(error),
        },
        _ => Err(ShowError::Ambiguous {
            title: title.to_string(),
            candidates: matches.into_iter().map(MatchedCandidate::summary).collect(),
        }),
    }
}

fn load_candidate(
    tree_dir: &Path,
    canonical_tree_dir: Option<&Path>,
    entry: &index::IndexEntry,
) -> CandidateLoad {
    let fallback_title = entry.title.trim().to_string();
    let fallback_url = non_empty_trimmed(&entry.url);
    let unresolved_summary = ShowCandidateSummary {
        file: entry.file.clone(),
        title: fallback_title.clone(),
        path: entry.file.clone(),
        url: fallback_url.clone(),
    };

    let path = match resolve_leaf_path(tree_dir, canonical_tree_dir, &entry.file) {
        Ok(path) => path,
        Err(_) => {
            return CandidateLoad::Broken {
                summary: unresolved_summary,
                error: ShowError::SuspiciousPath {
                    file: entry.file.clone(),
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
                    file: entry.file.clone(),
                },
            };
        }
        Err(e) => {
            return CandidateLoad::Broken {
                summary: fallback_summary,
                error: ShowError::UnreadableFile {
                    file: entry.file.clone(),
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
                    file: entry.file.clone(),
                    reason,
                },
            };
        }
    };

    let title = frontmatter_string(&document.frontmatter, "title")
        .or_else(|| non_empty_trimmed(&entry.title))
        .unwrap_or_default();
    let url = frontmatter_string(&document.frontmatter, "url").or(fallback_url);

    CandidateLoad::Loaded(LoadedLeaf {
        summary: ShowCandidateSummary {
            file: entry.file.clone(),
            title,
            path: path_string,
            url,
        },
        frontmatter: document.frontmatter,
        frontmatter_raw: document.frontmatter_raw,
        body: document.body,
    })
}

fn build_result(leaf: LoadedLeaf, options: &ShowOptions) -> ShowResult {
    let (body, truncated) = body_for_options(&leaf.body, options.full);

    ShowResult {
        title: leaf.summary.title,
        file: leaf.summary.file,
        path: leaf.summary.path,
        url: leaf.summary.url,
        frontmatter: leaf.frontmatter,
        frontmatter_raw: leaf.frontmatter_raw,
        body,
        truncated,
        full: options.full,
    }
}

fn resolve_leaf_path(
    tree_dir: &Path,
    canonical_tree_dir: Option<&Path>,
    file: &str,
) -> Result<PathBuf, &'static str> {
    let relative = Path::new(file);

    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || has_disallowed_components(relative)
    {
        return Err("suspicious path");
    }

    let resolved = tree_dir.join(relative);

    if let Some(canonical_root) = canonical_tree_dir {
        if resolved.exists() {
            let canonical_resolved = fs::canonicalize(&resolved).map_err(|_| "suspicious path")?;
            if !canonical_resolved.starts_with(canonical_root) {
                return Err("suspicious path");
            }
        } else if let Some(parent) = resolved.parent() {
            if parent.exists() {
                let canonical_parent = fs::canonicalize(parent).map_err(|_| "suspicious path")?;
                if !canonical_parent.starts_with(canonical_root) {
                    return Err("suspicious path");
                }
            }
        }
    }

    Ok(resolved)
}

#[cfg(windows)]
fn has_disallowed_components(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
}

#[cfg(not(windows))]
fn has_disallowed_components(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn parse_leaf_document(content: &str) -> Result<LeafDocument, String> {
    let rest = content
        .strip_prefix("---\n")
        .ok_or_else(|| "no frontmatter delimiters found".to_string())?;
    let close_pos = rest
        .find("\n---")
        .ok_or_else(|| "no frontmatter delimiters found".to_string())?;

    let yaml = &rest[..close_pos + 1];
    let frontmatter = serde_yaml_ng::from_str::<Mapping>(yaml).map_err(|e| e.to_string())?;

    let after_marker_start = "---\n".len() + close_pos + "\n---".len();
    let after_marker = &content[after_marker_start..];
    let raw_end = after_marker_start + usize::from(after_marker.starts_with('\n'));
    let after_closing_line = after_marker.strip_prefix('\n').unwrap_or(after_marker);
    let body = after_closing_line
        .strip_prefix('\n')
        .unwrap_or(after_closing_line)
        .to_string();

    Ok(LeafDocument {
        frontmatter,
        frontmatter_raw: content[..raw_end].to_string(),
        body,
    })
}

fn frontmatter_string(mapping: &Mapping, key: &str) -> Option<String> {
    mapping
        .get(key)
        .and_then(Value::as_str)
        .and_then(non_empty_trimmed)
}

fn non_empty_trimmed(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn normalize_title(title: &str) -> String {
    title.to_lowercase()
}

fn body_for_options(body: &str, full: bool) -> (String, bool) {
    if full {
        return (body.to_string(), false);
    }

    let char_count = body.chars().count();
    if char_count <= PREVIEW_CHAR_LIMIT {
        return (body.to_string(), false);
    }

    (body.chars().take(PREVIEW_CHAR_LIMIT).collect(), true)
}

// ── render ───────────────────────────────────────────────────────────────────

pub fn render_human(result: &ShowResult) -> String {
    let mut output = String::new();
    output.push_str(&result.frontmatter_raw);
    if !output.ends_with('\n') {
        output.push('\n');
    }
    output.push('\n');
    output.push_str(&result.body);
    if !result.body.ends_with('\n') {
        output.push('\n');
    }

    if result.truncated {
        output.push_str("\n[preview truncated; rerun with --full to show the complete leaf]\n");
    }

    output
}

#[derive(Serialize)]
struct ShowJsonPayload<'a> {
    leaf: &'a ShowResult,
}

pub fn render_json(result: &ShowResult) -> Result<String, ShowError> {
    serde_json::to_string_pretty(&ShowJsonPayload { leaf: result }).map_err(ShowError::from)
}

// ── internal types ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
struct LeafDocument {
    frontmatter: Mapping,
    frontmatter_raw: String,
    body: String,
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

#[derive(Debug)]
enum MatchedCandidate {
    Loaded(LoadedLeaf),
    Broken {
        summary: ShowCandidateSummary,
        error: ShowError,
    },
}

impl MatchedCandidate {
    fn summary(self) -> ShowCandidateSummary {
        match self {
            MatchedCandidate::Loaded(leaf) => leaf.summary,
            MatchedCandidate::Broken { summary, .. } => summary,
        }
    }
}

#[cfg(test)]
#[path = "../tests/cli_show_tests.rs"]
mod tests;
