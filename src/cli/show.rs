// bo show — deterministic inspection for a single collected leaf.

use crate::cli::json::JsonError;
use crate::domain::manifest::{self, LeafRecord};
use crate::domain::tree::Tree;
use serde::Serialize;
use serde_json::json;
use serde_yaml_ng::{Mapping, Value};
use std::fmt;
use std::fs;
use std::io::{self, ErrorKind};
use std::path::{Component, Path, PathBuf};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    pub full: bool,
}

#[derive(Debug)]
pub enum ShowError {
    Io(io::Error),
    Json(serde_json::Error),
    Manifest(manifest::ManifestError),
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
            ShowError::Manifest(e) => write!(f, "{}", e),
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

impl ShowError {
    fn code(&self) -> &'static str {
        match self {
            ShowError::SuspiciousPath { .. } => "suspicious_path",
            ShowError::MissingFile { .. } => "not_found",
            ShowError::InvalidFrontmatter { .. } => "validation_error",
            _ => "unknown_error",
        }
    }

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
            ShowError::Json(_) => JsonError::new("json_error", self.to_string()),
            ShowError::Manifest(_) => JsonError::new("manifest_error", self.to_string()),
            ShowError::SuspiciousPath { file }
            | ShowError::MissingFile { file }
            | ShowError::InvalidFrontmatter { file, .. } => {
                JsonError::with_details(self.code(), self.to_string(), json!({ "file": file }))
            }
            ShowError::UnreadableFile { file, source: _ } => {
                JsonError::with_details("io_error", self.to_string(), json!({ "file": file }))
            }
        }
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

    let tree = Tree {
        name: "unnamed".to_string(),
        created_at: None,
        output_dir: tree_dir.to_path_buf(),
    };
    let manifest = match manifest::read(&tree.manifest_path()) {
        Ok(m) => m,
        Err(manifest::ManifestError::TreeNotInitialized) => {
            return Err(ShowError::NotFound {
                title: title.to_string(),
            });
        }
        Err(e) => return Err(ShowError::Manifest(e)),
    };
    let canonical_tree_dir = fs::canonicalize(tree_dir).ok();

    let mut matches = Vec::new();
    for leaf in &manifest.leaves {
        match load_candidate(tree_dir, canonical_tree_dir.as_deref(), leaf) {
            CandidateLoad::Loaded(loaded) => {
                if normalize_title(&loaded.summary.title) == requested_title {
                    matches.push(MatchedCandidate::Loaded(loaded));
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
    leaf: &LeafRecord,
) -> CandidateLoad {
    let fallback_title = leaf.title.as_str().trim().to_string();
    let fallback_url = non_empty_trimmed(leaf.url.as_str());
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

    let title = frontmatter_string(&document.frontmatter, "title")
        .or_else(|| non_empty_trimmed(leaf.title.as_str()))
        .unwrap_or_default();
    let url = frontmatter_string(&document.frontmatter, "url").or(fallback_url);

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

fn body_for_options(body: &str, full: bool) -> (Option<String>, Option<bool>) {
    if full {
        return (Some(body.to_string()), None);
    }
    // Card view: frontmatter only, no body.
    (None, None)
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

        if result.truncated == Some(true) {
            output.push_str("\n[preview truncated; rerun with --full to show the complete leaf]\n");
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
