use bo::cli::collect::{self, CollectError};
use bo::cli::compile::{self, BranchResult, CompileError, CompileResult};
use bo::cli::json::{self as json_output, JsonError, JsonWarning};
use bo::cli::list::{self, ListOptions};
use bo::cli::search::{self, SearchOptions, SearchQuery};
use bo::cli::show::{self, ShowOptions};
use bo::domain::index;
use bo::engine::config::{self, Config, ConfigError};

use chrono::Utc;
use clap::{error::ErrorKind as ClapErrorKind, Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, ErrorKind as IoErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::process;

const NOT_SEEDED_MSG: &str = "bo hasn't been seeded yet — run: bo seed <output-dir>";
const KNOWN_COMMANDS: &[&str] = &[
    "seed", "collect", "compile", "list", "search", "show", "raze",
];

#[derive(Parser, Debug)]
#[command(name = "bo", about = "Collect web pages into a local markdown tree")]
struct Cli {
    /// Emit machine-readable JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialise a tree at <output-dir> and save config
    Seed {
        /// Directory to store collected content
        output_dir: PathBuf,
        /// Human-readable name for the tree (defaults to the output directory basename)
        #[arg(long)]
        name: Option<String>,
    },
    /// Fetch a URL and collect it
    Collect {
        /// URL to collect
        url: String,
    },
    /// Compile collected documents into a linked knowledge graph
    Compile,
    /// List collected leaves in the current tree
    List {
        /// Maximum number of leaves to show
        #[arg(long)]
        limit: Option<usize>,
        /// Sort by collected date, newest first
        #[arg(long)]
        recent: bool,
        /// Filter by exact branch name/slug
        #[arg(long)]
        branch: Option<String>,
    },
    /// Search collected leaves by content
    Search {
        /// Search terms (all must match). Quote phrases: "borrow checker"
        #[arg(required = true)]
        terms: Vec<String>,
        /// Page number (default 1, 5 results per page)
        #[arg(long, default_value = "1")]
        page: usize,
        /// Sort by collected date instead of relevance
        #[arg(long)]
        recent: bool,
    },
    /// Show one collected leaf by exact title
    Show {
        /// Leaf title to show
        title: String,
        /// Show the full leaf body instead of a preview
        #[arg(long)]
        full: bool,
    },
    /// Delete all bo-managed files and config
    Raze,
}

// ── JSON payloads ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct SeedResult {
    status: String,
    output_dir: String,
    tree_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct CollectResult {
    url: String,
    file: String,
    path: String,
}

#[derive(Debug, Clone, Serialize)]
struct SearchJsonData<'a> {
    query: SearchJsonQuery,
    #[serde(flatten)]
    result: &'a search::SearchResult,
}

#[derive(Debug, Clone, Serialize)]
struct SearchJsonQuery {
    terms: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct ShowJsonData<'a> {
    leaf: &'a show::ShowResult,
}

#[derive(Debug, Clone, Serialize)]
struct RazeResult {
    deleted_files: usize,
    deleted_index: bool,
    removed_output_dir: bool,
    output_dir_left_in_place: bool,
    deleted_config: bool,
    output_dir: String,
    config_path: String,
}

#[derive(Debug)]
struct RazeOutput {
    result: RazeResult,
    warnings: Vec<JsonWarning>,
}

// ── errors ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum CliError {
    Usage { message: String, exit_code: i32 },
    NotSeeded,
    ConfigRead(String),
    ConfigWrite(String),
    CurrentDir(String),
    CreateOutputDir(String),
    Collect(CollectError),
    List(list::ListError),
    Search(search::SearchError),
    Show(show::ShowError),
    Compile(CompileError),
    IoMessage(String),
}

impl CliError {
    fn exit_code(&self) -> i32 {
        match self {
            CliError::Usage { exit_code, .. } => *exit_code,
            _ => 1,
        }
    }

    fn to_json_error(&self) -> JsonError {
        match self {
            CliError::Usage { message, .. } => JsonError::with_details(
                "usage_error",
                message.clone(),
                json!({ "exit_code": self.exit_code() }),
            ),
            CliError::NotSeeded => JsonError::new("not_seeded", NOT_SEEDED_MSG),
            CliError::ConfigRead(message)
            | CliError::ConfigWrite(message)
            | CliError::CurrentDir(message)
            | CliError::CreateOutputDir(message)
            | CliError::IoMessage(message) => JsonError::new("io_error", message.clone()),
            CliError::Collect(error) => collect_json_error(error),
            CliError::List(error) => JsonError::new(list_error_code(error), error.to_string()),
            CliError::Search(error) => JsonError::new(search_error_code(error), error.to_string()),
            CliError::Show(error) => show_json_error(error),
            CliError::Compile(error) => compile_json_error(error),
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Usage { message, .. } => write!(f, "{}", message),
            CliError::NotSeeded => write!(f, "{}", NOT_SEEDED_MSG),
            CliError::ConfigRead(message)
            | CliError::ConfigWrite(message)
            | CliError::CurrentDir(message)
            | CliError::CreateOutputDir(message)
            | CliError::IoMessage(message) => write!(f, "{}", message),
            CliError::Collect(error) => write!(f, "{}", error),
            CliError::List(error) => write!(f, "{}", error),
            CliError::Search(error) => write!(f, "{}", error),
            CliError::Show(error) => write!(f, "{}", error),
            CliError::Compile(error) => write!(f, "{}", error),
        }
    }
}

fn collect_json_error(error: &CollectError) -> JsonError {
    match error {
        CollectError::DuplicateUrl { existing_file } => JsonError::with_details(
            "duplicate_url",
            error.to_string(),
            json!({ "existing_file": existing_file }),
        ),
        CollectError::Rejected { url, reason } => JsonError::with_details(
            "rejected",
            error.to_string(),
            json!({ "url": url, "reason": reason.to_string() }),
        ),
        CollectError::Fetch(_) => JsonError::new("fetch_error", error.to_string()),
        CollectError::Extract(_) => JsonError::new("extract_error", error.to_string()),
        CollectError::Youtube(_) => JsonError::new("youtube_error", error.to_string()),
        CollectError::Io(_) => JsonError::new("io_error", error.to_string()),
    }
}

fn list_error_code(error: &list::ListError) -> &'static str {
    match error {
        list::ListError::Io(_) => "io_error",
        list::ListError::Json(_) => "json_error",
    }
}

fn search_error_code(error: &search::SearchError) -> &'static str {
    match error {
        search::SearchError::Io(_) => "io_error",
        search::SearchError::Json(_) => "json_error",
    }
}

fn show_json_error(error: &show::ShowError) -> JsonError {
    match error {
        show::ShowError::NotFound { title } => {
            JsonError::with_details("not_found", error.to_string(), json!({ "title": title }))
        }
        show::ShowError::Ambiguous { title, candidates } => JsonError::with_details(
            "ambiguous",
            error.to_string(),
            json!({ "title": title, "candidates": candidates }),
        ),
        show::ShowError::Io(_) => JsonError::new("io_error", error.to_string()),
        show::ShowError::Json(_) => JsonError::new("json_error", error.to_string()),
        show::ShowError::SuspiciousPath { file }
        | show::ShowError::MissingFile { file }
        | show::ShowError::InvalidFrontmatter { file, .. } => JsonError::with_details(
            show_error_code(error),
            error.to_string(),
            json!({ "file": file }),
        ),
        show::ShowError::UnreadableFile { file, source: _ } => {
            JsonError::with_details("io_error", error.to_string(), json!({ "file": file }))
        }
    }
}

fn show_error_code(error: &show::ShowError) -> &'static str {
    match error {
        show::ShowError::SuspiciousPath { .. } => "suspicious_path",
        show::ShowError::MissingFile { .. } => "not_found",
        show::ShowError::InvalidFrontmatter { .. } => "validation_error",
        _ => "unknown_error",
    }
}

fn compile_json_error(error: &CompileError) -> JsonError {
    let code = match error {
        CompileError::ContextOverflow => "context_overflow",
        CompileError::Truncated => "truncated",
        CompileError::ContentFilter => "content_filter",
        CompileError::Llm(_) => "llm_error",
        CompileError::Io(_) => "io_error",
        CompileError::Validation(_) => "validation_error",
    };
    JsonError::new(code, error.to_string())
}

// ── runner ───────────────────────────────────────────────────────────────────

fn run_from<I, T, W, E>(args: I, stdout: &mut W, stderr: &mut E) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
    W: Write,
    E: Write,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    let raw_json_mode = raw_json_mode_requested(&args);

    match Cli::try_parse_from(args.clone()) {
        Ok(cli) => run_cli(cli, stdout, stderr),
        Err(error) => render_parse_error(error, raw_json_mode, &args, stdout, stderr),
    }
}

fn run_cli<W: Write, E: Write>(cli: Cli, stdout: &mut W, stderr: &mut E) -> i32 {
    let json = cli.json;

    match cli.command {
        Commands::Seed { output_dir, name } => match execute_seed(output_dir, name) {
            Ok(result) if json => emit_json_success("seed", &result, Vec::new(), stdout),
            Ok(result) => write_human_or_error(render_seed_human(&result, stdout), stderr),
            Err(error) => emit_cli_error("seed", json, error, stdout, stderr),
        },
        Commands::Collect { url } => match execute_collect(url) {
            Ok(result) if json => emit_json_success("collect", &result, Vec::new(), stdout),
            Ok(result) => write_human_or_error(render_collect_human(&result, stdout), stderr),
            Err(error) => emit_cli_error("collect", json, error, stdout, stderr),
        },
        Commands::Compile => match require_config()
            .and_then(|cfg| compile::run_compile(&cfg).map_err(CliError::Compile))
        {
            Ok(result) if json => {
                let warnings = compile_warnings(&result);
                emit_json_success("compile", &result, warnings, stdout)
            }
            Ok(result) => write_human_or_error(render_compile_human(&result, stdout), stderr),
            Err(error) => emit_cli_error("compile", json, error, stdout, stderr),
        },
        Commands::List {
            limit,
            recent,
            branch,
        } => match execute_list(limit, recent, branch) {
            Ok(result) if json => {
                let warnings = list_warnings(&result);
                emit_json_success("list", &result, warnings, stdout)
            }
            Ok(result) => {
                write_human_or_error(write!(stdout, "{}", list::render_human(&result)), stderr)
            }
            Err(error) => emit_cli_error("list", json, error, stdout, stderr),
        },
        Commands::Search {
            terms,
            page,
            recent,
        } => match execute_search(terms, page, recent) {
            Ok((query, result)) if json => {
                let data = SearchJsonData {
                    query: SearchJsonQuery { terms: query.terms },
                    result: &result,
                };
                emit_json_success("search", data, Vec::new(), stdout)
            }
            Ok((_query, result)) => {
                let has_results = !result.hits.is_empty();
                let code = write_human_or_error(
                    write!(stdout, "{}", search::render_human(&result)),
                    stderr,
                );
                if code != 0 {
                    code
                } else if has_results {
                    0
                } else {
                    1
                }
            }
            Err(error) => emit_cli_error("search", json, error, stdout, stderr),
        },
        Commands::Show { title, full } => match execute_show(title, full) {
            Ok(result) if json => {
                emit_json_success("show", ShowJsonData { leaf: &result }, Vec::new(), stdout)
            }
            Ok(result) => {
                write_human_or_error(write!(stdout, "{}", show::render_human(&result)), stderr)
            }
            Err(error) => emit_cli_error("show", json, error, stdout, stderr),
        },
        Commands::Raze => match execute_raze() {
            Ok(output) if json => {
                emit_json_success("raze", &output.result, output.warnings, stdout)
            }
            Ok(output) => {
                for warning in &output.warnings {
                    let _ = writeln!(stderr, "warning: {}", warning.message);
                }
                write_human_or_error(render_raze_human(&output.result, stdout), stderr)
            }
            Err(error) => emit_cli_error("raze", json, error, stdout, stderr),
        },
    }
}

fn render_parse_error<W: Write, E: Write>(
    error: clap::Error,
    raw_json_mode: bool,
    args: &[OsString],
    stdout: &mut W,
    stderr: &mut E,
) -> i32 {
    let exit_code = error.exit_code();

    if matches!(
        error.kind(),
        ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion
    ) {
        return write_rendered(stdout, &error.render().to_string(), exit_code);
    }

    if raw_json_mode {
        let command = infer_command(args);
        let rendered = error.render().to_string();
        let json_error = JsonError::with_details(
            "usage_error",
            rendered.trim().to_string(),
            json!({
                "kind": format!("{:?}", error.kind()),
                "exit_code": exit_code,
            }),
        );
        return emit_json_error(command, json_error, Vec::new(), stdout, exit_code);
    }

    write_rendered(stderr, &error.render().to_string(), exit_code)
}

fn raw_json_mode_requested(args: &[OsString]) -> bool {
    args.iter()
        .skip(1)
        .take_while(|arg| arg.as_os_str() != "--")
        .any(|arg| arg.as_os_str() == "--json")
}

fn infer_command(args: &[OsString]) -> &'static str {
    for arg in args.iter().skip(1) {
        if arg.as_os_str() == "--" {
            break;
        }

        let Some(value) = arg.to_str() else {
            continue;
        };

        if value == "--json" || value.starts_with('-') {
            continue;
        }

        if let Some(command) = KNOWN_COMMANDS
            .iter()
            .copied()
            .find(|command| *command == value)
        {
            return command;
        }

        return "bo";
    }

    "bo"
}

fn write_rendered<W: Write>(writer: &mut W, rendered: &str, exit_code: i32) -> i32 {
    match write!(writer, "{}", rendered) {
        Ok(()) => exit_code,
        Err(_) => 1,
    }
}

fn emit_json_success<W: Write, T: Serialize>(
    command: &str,
    data: T,
    warnings: Vec<JsonWarning>,
    stdout: &mut W,
) -> i32 {
    match json_output::success_string(command, data, warnings) {
        Ok(encoded) => match writeln!(stdout, "{}", encoded) {
            Ok(()) => 0,
            Err(_) => 1,
        },
        Err(error) => emit_json_error(
            command,
            JsonError::new(
                "json_error",
                format!("failed to serialize JSON response: {error}"),
            ),
            Vec::new(),
            stdout,
            1,
        ),
    }
}

fn emit_json_error<W: Write>(
    command: &str,
    error: JsonError,
    warnings: Vec<JsonWarning>,
    stdout: &mut W,
    exit_code: i32,
) -> i32 {
    match json_output::error_string(command, error, warnings) {
        Ok(encoded) => match writeln!(stdout, "{}", encoded) {
            Ok(()) => exit_code,
            Err(_) => 1,
        },
        Err(_) => 1,
    }
}

fn emit_cli_error<W: Write, E: Write>(
    command: &str,
    json: bool,
    error: CliError,
    stdout: &mut W,
    stderr: &mut E,
) -> i32 {
    let exit_code = error.exit_code();
    if json {
        return emit_json_error(
            command,
            error.to_json_error(),
            Vec::new(),
            stdout,
            exit_code,
        );
    }

    match writeln!(stderr, "error: {}", error) {
        Ok(()) => exit_code,
        Err(_) => 1,
    }
}

fn write_human_or_error<E: Write>(result: io::Result<()>, _stderr: &mut E) -> i32 {
    match result {
        Ok(()) => 0,
        Err(_) => 1,
    }
}

// ── command execution ────────────────────────────────────────────────────────

fn require_config() -> Result<Config, CliError> {
    match config::read_config(&config::config_path()) {
        Ok(cfg) => Ok(cfg),
        Err(ConfigError::NotFound) => Err(CliError::NotSeeded),
        Err(error) => Err(CliError::ConfigRead(format!(
            "failed to read config: {}",
            error
        ))),
    }
}

fn execute_seed(output_dir: PathBuf, name: Option<String>) -> Result<SeedResult, CliError> {
    let output_dir = if output_dir.is_absolute() {
        output_dir
    } else {
        std::env::current_dir()
            .map_err(|error| CliError::CurrentDir(format!("failed to get current dir: {error}")))?
            .join(&output_dir)
    };

    match config::read_config(&config::config_path()) {
        Ok(existing) => {
            return Ok(SeedResult {
                status: "already_seeded".to_string(),
                output_dir: path_string(&existing.tree.output_dir),
                tree_name: existing.tree.name,
            });
        }
        Err(ConfigError::NotFound) => {}
        Err(error) => {
            return Err(CliError::ConfigRead(format!(
                "failed to read config: {}",
                error
            )));
        }
    }

    std::fs::create_dir_all(&output_dir).map_err(|error| {
        CliError::CreateOutputDir(format!("failed to create output directory: {error}"))
    })?;

    let tree_name = name.or_else(|| {
        output_dir
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    });

    let created_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    config::write_config(
        &Config {
            tree: bo::domain::tree::TreeConfig {
                output_dir: output_dir.clone(),
                name: tree_name.clone(),
                created_at: Some(created_at),
            },
            compile_model: None,
        },
        &config::config_path(),
    )
    .map_err(|error| CliError::ConfigWrite(format!("failed to write config: {error}")))?;

    Ok(SeedResult {
        status: "created".to_string(),
        output_dir: path_string(&output_dir),
        tree_name,
    })
}

fn execute_collect(url: String) -> Result<CollectResult, CliError> {
    let cfg = require_config()?;
    eprintln!("fetching {}...", url);
    let page = collect::collect_url(&url, &cfg.tree.output_dir).map_err(CliError::Collect)?;
    let path = cfg.tree.output_dir.join(&page.filename);

    Ok(CollectResult {
        url: page.url,
        file: page.filename,
        path: path_string(&path),
    })
}

fn execute_list(
    limit: Option<usize>,
    recent: bool,
    branch: Option<String>,
) -> Result<list::ListResult, CliError> {
    let cfg = require_config()?;
    list::list_leaves(
        &cfg.tree.output_dir,
        &ListOptions {
            limit,
            recent,
            branch,
        },
    )
    .map_err(CliError::List)
}

fn execute_search(
    terms: Vec<String>,
    page: usize,
    recent: bool,
) -> Result<(SearchQuery, search::SearchResult), CliError> {
    if page == 0 {
        return Err(CliError::Usage {
            message: "--page must be at least 1".to_string(),
            exit_code: 2,
        });
    }

    let cfg = require_config()?;
    let query = SearchQuery {
        terms: terms.iter().map(|term| term.to_lowercase()).collect(),
    };
    let options = SearchOptions { page, recent };
    let result =
        search::search_leaves(&cfg.tree.output_dir, &query, &options).map_err(CliError::Search)?;

    Ok((query, result))
}

fn execute_show(title: String, full: bool) -> Result<show::ShowResult, CliError> {
    let cfg = require_config()?;
    show::show_leaf(&cfg.tree.output_dir, &title, &ShowOptions { full }).map_err(CliError::Show)
}

fn execute_raze() -> Result<RazeOutput, CliError> {
    let cfg = require_config()?;
    let output_dir = cfg.tree.output_dir;
    let index_path = output_dir.join("index.jsonl");
    let entries = index::read_index(&index_path)
        .map_err(|error| CliError::IoMessage(format!("failed to read index: {error}")))?;

    let mut deleted_files = 0usize;
    let mut warnings = Vec::new();

    for entry in &entries {
        if is_suspicious_relative_path(&entry.file) {
            warnings.push(JsonWarning::with_details(
                "suspicious_ledger_entry",
                format!("skipping ledger entry with suspicious path: {}", entry.file),
                json!({ "file": entry.file }),
            ));
            continue;
        }

        let resolved = output_dir.join(&entry.file);

        match std::fs::remove_file(&resolved) {
            Ok(()) => deleted_files += 1,
            Err(error) if error.kind() == IoErrorKind::NotFound => {}
            Err(error) => {
                return Err(CliError::IoMessage(format!(
                    "failed to delete {}: {}",
                    resolved.display(),
                    error
                )));
            }
        }
    }

    let deleted_index = match std::fs::remove_file(&index_path) {
        Ok(()) => true,
        Err(error) if error.kind() == IoErrorKind::NotFound => false,
        Err(error) => {
            return Err(CliError::IoMessage(format!(
                "failed to delete ledger: {}",
                error
            )));
        }
    };

    let (removed_output_dir, output_dir_left_in_place) = match std::fs::remove_dir(&output_dir) {
        Ok(()) => (true, false),
        Err(error)
            if error.kind() == IoErrorKind::DirectoryNotEmpty
                || error.kind() == IoErrorKind::NotFound =>
        {
            (false, true)
        }
        Err(error) => {
            return Err(CliError::IoMessage(format!(
                "failed to remove output directory: {}",
                error
            )));
        }
    };

    let config_path = config::config_path();
    let deleted_config = match std::fs::remove_file(&config_path) {
        Ok(()) => true,
        Err(error) if error.kind() == IoErrorKind::NotFound => false,
        Err(error) => {
            return Err(CliError::IoMessage(format!(
                "failed to delete config: {}",
                error
            )));
        }
    };

    Ok(RazeOutput {
        result: RazeResult {
            deleted_files,
            deleted_index,
            removed_output_dir,
            output_dir_left_in_place,
            deleted_config,
            output_dir: path_string(&output_dir),
            config_path: path_string(&config_path),
        },
        warnings,
    })
}

fn path_string(path: &Path) -> String {
    path.display().to_string()
}

fn is_suspicious_relative_path(file: &str) -> bool {
    let relative = Path::new(file);
    relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
}

// ── human rendering ──────────────────────────────────────────────────────────

fn render_seed_human<W: Write>(result: &SeedResult, stdout: &mut W) -> io::Result<()> {
    match result.status.as_str() {
        "already_seeded" => writeln!(
            stdout,
            "bo has already been seeded at {}!",
            result.output_dir
        ),
        _ => writeln!(stdout, "seeded bo at {}", result.output_dir),
    }
}

fn render_collect_human<W: Write>(result: &CollectResult, stdout: &mut W) -> io::Result<()> {
    writeln!(stdout, "✓ collected: {} → {}", result.url, result.file)
}

fn render_compile_human<W: Write>(result: &CompileResult, stdout: &mut W) -> io::Result<()> {
    if result.status == "noop" {
        match result.reason.as_deref() {
            Some("empty_tree") => writeln!(stdout, "bo is empty!"),
            Some("single_leaf") => writeln!(stdout, "bo only has 1 leaf!"),
            _ => writeln!(stdout, "compiled: no work to do"),
        }?;
        return Ok(());
    }

    render_compile_summary_human(
        &result.branches,
        result.leaves_updated,
        &result.leaves_skipped,
        stdout,
    )
}

fn render_compile_summary_human<W: Write>(
    branches: &[BranchResult],
    leaves_updated: usize,
    leaves_skipped: &[String],
    stdout: &mut W,
) -> io::Result<()> {
    if branches.is_empty() {
        writeln!(stdout, "compiled: no branches found")?;
    } else {
        writeln!(
            stdout,
            "compiled: {} {} across {} leaves",
            branches.len(),
            if branches.len() == 1 {
                "branch"
            } else {
                "branches"
            },
            leaves_updated
        )?;
        for branch in branches {
            writeln!(
                stdout,
                "  ✓ {} ({} {})",
                branch.slug,
                branch.leaf_count,
                if branch.leaf_count == 1 {
                    "leaf"
                } else {
                    "leaves"
                }
            )?;
        }
    }

    if !leaves_skipped.is_empty() {
        writeln!(stdout)?;
        writeln!(
            stdout,
            "  ⚠ skipped {} {} (unparseable frontmatter):",
            leaves_skipped.len(),
            if leaves_skipped.len() == 1 {
                "leaf"
            } else {
                "leaves"
            }
        )?;
        for file in leaves_skipped {
            writeln!(stdout, "    - {}", file)?;
        }
    }

    Ok(())
}

fn render_raze_human<W: Write>(result: &RazeResult, stdout: &mut W) -> io::Result<()> {
    writeln!(stdout, "deleted {} markdown file(s)", result.deleted_files)?;

    if result.deleted_index {
        writeln!(stdout, "deleted index")?;
    }

    if result.removed_output_dir {
        writeln!(stdout, "removed output directory {}", result.output_dir)?;
    } else if result.output_dir_left_in_place {
        writeln!(
            stdout,
            "output directory left in place (not empty or already absent): {}",
            result.output_dir
        )?;
    }

    if result.deleted_config {
        writeln!(stdout, "deleted config")?;
    }

    Ok(())
}

// ── warning extraction ───────────────────────────────────────────────────────

fn list_warnings(result: &list::ListResult) -> Vec<JsonWarning> {
    result
        .leaves
        .iter()
        .filter(|row| row.degraded)
        .map(|row| {
            JsonWarning::with_details(
                "degraded_leaf",
                format!("leaf '{}' is degraded", row.display_title),
                json!({
                    "file": row.file,
                    "reasons": row.degradation_reasons,
                }),
            )
        })
        .collect()
}

fn compile_warnings(result: &CompileResult) -> Vec<JsonWarning> {
    if result.leaves_skipped.is_empty() {
        return Vec::new();
    }

    vec![JsonWarning::with_details(
        "skipped_leaves",
        format!(
            "skipped {} leaves with unparseable frontmatter",
            result.leaves_skipped.len()
        ),
        json!({ "files": result.leaves_skipped }),
    )]
}

// ── compatibility wrappers used by existing unit tests ───────────────────────

#[cfg(test)]
fn cmd_seed(output_dir: PathBuf, name: Option<String>) -> Result<(), String> {
    let result = execute_seed(output_dir, name).map_err(|error| error.to_string())?;
    render_seed_human(&result, &mut io::stdout()).map_err(|error| error.to_string())
}

// ── main ─────────────────────────────────────────────────────────────────────

fn main() {
    let _ = dotenvy::dotenv();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_target(false)
        .without_time()
        .with_level(false)
        .init();

    let mut stdout = io::stdout();
    let mut stderr = io::stderr();
    let exit_code = run_from(std::env::args_os(), &mut stdout, &mut stderr);
    process::exit(exit_code);
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bo::engine::config;
    use serde_json::Value;
    use std::fs;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static HOME_LOCK: Mutex<()> = Mutex::new(());

    fn seed_with_home(
        home: &TempDir,
        output_dir: PathBuf,
        name: Option<String>,
    ) -> Result<(), String> {
        let _guard = HOME_LOCK.lock().unwrap();
        std::env::set_var("HOME", home.path());
        cmd_seed(output_dir, name)
    }

    fn run_with_home(home: &TempDir, args: &[&str]) -> (i32, String, String) {
        let _guard = HOME_LOCK.lock().unwrap();
        std::env::set_var("HOME", home.path());
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let exit = run_from(args.iter().copied(), &mut stdout, &mut stderr);
        (
            exit,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    fn parse_json(stdout: &str) -> Value {
        serde_json::from_str(stdout)
            .unwrap_or_else(|error| panic!("stdout is not valid JSON: {error}\nstdout:\n{stdout}"))
    }

    fn seed_tree(home: &TempDir, name: &str) -> PathBuf {
        let output_dir = home.path().join(name);
        let output_arg = output_dir.to_string_lossy().to_string();
        let (exit, _stdout, _stderr) = run_with_home(home, &["bo", "seed", &output_arg]);
        assert_eq!(exit, 0);
        output_dir
    }

    fn write_compile_leaf(tree: &Path, file: &str, title: &str) {
        bo::domain::index::append_entry(
            &tree.join("index.jsonl"),
            &bo::domain::index::IndexEntry {
                file: file.to_string(),
                title: title.to_string(),
                url: format!("https://example.com/{}", file.trim_end_matches(".md")),
            },
        )
        .unwrap();
        fs::write(
            tree.join(file),
            format!(
                "---\ntitle: {title}\nurl: https://example.com/{slug}\ncollected_at: 2025-01-01T00:00:00Z\nupdated_at: 2025-01-01T00:00:00Z\n---\n\n# {title}\n\nBody.\n",
                slug = file.trim_end_matches(".md")
            ),
        )
        .unwrap();
    }

    #[test]
    fn raw_json_mode_detection_stops_at_arg_terminator() {
        assert!(raw_json_mode_requested(&[
            OsString::from("bo"),
            OsString::from("search"),
            OsString::from("--json"),
        ]));
        assert!(!raw_json_mode_requested(&[
            OsString::from("bo"),
            OsString::from("search"),
            OsString::from("--"),
            OsString::from("--json"),
        ]));
    }

    #[test]
    fn json_parser_error_for_missing_subcommand() {
        let home = TempDir::new().unwrap();
        let (exit, stdout, stderr) = run_with_home(&home, &["bo", "--json"]);
        assert_ne!(exit, 0);
        assert!(stderr.is_empty());
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["command"], "bo");
        assert_eq!(parsed["error"]["code"], "usage_error");
    }

    #[test]
    fn command_local_json_flag_is_accepted() {
        let home = TempDir::new().unwrap();
        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "list", "--json"]);
        assert_ne!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["command"], "list");
        assert_eq!(parsed["error"]["code"], "not_seeded");
    }

    #[test]
    fn global_json_flag_is_accepted() {
        let home = TempDir::new().unwrap();
        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "--json", "list"]);
        assert_ne!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["command"], "list");
        assert_eq!(parsed["error"]["code"], "not_seeded");
    }

    #[test]
    fn seed_json_created_payload() {
        let home = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();
        let output_dir = output.path().join("my-tree");
        let output_arg = output_dir.to_string_lossy().to_string();

        let (exit, stdout, stderr) = run_with_home(&home, &["bo", "seed", "--json", &output_arg]);
        assert_eq!(exit, 0);
        assert!(stderr.is_empty());
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["command"], "seed");
        assert_eq!(parsed["data"]["status"], "created");
        assert_eq!(parsed["data"]["tree_name"], "my-tree");
    }

    #[test]
    fn seed_json_already_seeded_payload() {
        let home = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();
        let output_dir = output.path().join("my-tree");
        let output_arg = output_dir.to_string_lossy().to_string();

        let first = run_with_home(&home, &["bo", "seed", &output_arg]);
        assert_eq!(first.0, 0);
        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "seed", "--json", &output_arg]);
        assert_eq!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["data"]["status"], "already_seeded");
        assert_eq!(parsed["data"]["tree_name"], "my-tree");
    }

    #[test]
    fn every_output_command_accepts_json_flag() {
        let output = TempDir::new().unwrap();
        let output_arg = output.path().join("tree").to_string_lossy().to_string();

        let cases: Vec<(Vec<&str>, &str)> = vec![
            (vec!["bo", "seed", "--json", &output_arg], "seed"),
            (
                vec!["bo", "collect", "--json", "https://example.com"],
                "collect",
            ),
            (vec!["bo", "compile", "--json"], "compile"),
            (vec!["bo", "list", "--json"], "list"),
            (vec!["bo", "search", "--json", "term"], "search"),
            (vec!["bo", "show", "--json", "Title"], "show"),
            (vec!["bo", "raze", "--json"], "raze"),
        ];

        for (args, command) in cases {
            let home = TempDir::new().unwrap();
            let (_exit, stdout, _stderr) = run_with_home(&home, &args);
            let parsed = parse_json(&stdout);
            assert_eq!(parsed["command"], command, "args: {args:?}");
            assert!(parsed.get("schema_version").is_some(), "args: {args:?}");
            assert!(parsed.get("warnings").is_some(), "args: {args:?}");
        }
    }

    #[test]
    fn search_json_no_results_exits_successfully() {
        let home = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();
        let output_dir = output.path().join("tree");
        let output_arg = output_dir.to_string_lossy().to_string();
        let _ = run_with_home(&home, &["bo", "seed", &output_arg]);

        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "search", "--json", "missing"]);
        assert_eq!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["command"], "search");
        assert_eq!(parsed["data"]["hits"].as_array().unwrap().len(), 0);
        assert_eq!(parsed["data"]["query"]["terms"][0], "missing");
    }

    #[test]
    fn compile_json_empty_tree_is_noop_without_api_key() {
        let home = TempDir::new().unwrap();
        let tree = seed_tree(&home, "tree");
        assert!(tree.exists());
        std::env::remove_var("OPENAI_API_KEY");

        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "compile", "--json"]);
        assert_eq!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["data"]["status"], "noop");
        assert_eq!(parsed["data"]["reason"], "empty_tree");
    }

    #[test]
    fn compile_json_single_leaf_is_noop_without_api_key() {
        let home = TempDir::new().unwrap();
        let tree = seed_tree(&home, "tree");
        write_compile_leaf(&tree, "a.md", "A");
        std::env::remove_var("OPENAI_API_KEY");

        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "compile", "--json"]);
        assert_eq!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["data"]["status"], "noop");
        assert_eq!(parsed["data"]["reason"], "single_leaf");
    }

    #[test]
    fn compile_json_missing_api_key_is_structured_error() {
        let home = TempDir::new().unwrap();
        let tree = seed_tree(&home, "tree");
        write_compile_leaf(&tree, "a.md", "A");
        write_compile_leaf(&tree, "b.md", "B");
        std::env::remove_var("OPENAI_API_KEY");

        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "compile", "--json"]);
        assert_ne!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["error"]["code"], "io_error");
        assert!(parsed["error"]["message"]
            .as_str()
            .unwrap()
            .contains("OPENAI_API_KEY"));
    }

    #[test]
    fn show_json_not_found_is_structured_error() {
        let home = TempDir::new().unwrap();
        let _tree = seed_tree(&home, "tree");

        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "show", "--json", "Missing"]);
        assert_ne!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["command"], "show");
        assert_eq!(parsed["error"]["code"], "not_found");
        assert_eq!(parsed["error"]["details"]["title"], "Missing");
    }

    #[test]
    fn show_json_ambiguous_title_includes_candidates() {
        let home = TempDir::new().unwrap();
        let tree = seed_tree(&home, "tree");
        write_compile_leaf(&tree, "a.md", "Same Title");
        write_compile_leaf(&tree, "b.md", "Same Title");

        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "show", "--json", "Same Title"]);
        assert_ne!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["command"], "show");
        assert_eq!(parsed["error"]["code"], "ambiguous");
        assert_eq!(
            parsed["error"]["details"]["candidates"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn search_json_page_zero_is_structured_usage_error() {
        let home = TempDir::new().unwrap();
        let _tree = seed_tree(&home, "tree");

        let (exit, stdout, _stderr) =
            run_with_home(&home, &["bo", "search", "--json", "term", "--page", "0"]);
        assert_eq!(exit, 2);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["command"], "search");
        assert_eq!(parsed["error"]["code"], "usage_error");
    }

    #[test]
    fn raze_json_summary() {
        let home = TempDir::new().unwrap();
        let tree = seed_tree(&home, "tree");

        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "raze", "--json"]);
        assert_eq!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], true);
        assert_eq!(parsed["command"], "raze");
        assert_eq!(parsed["data"]["output_dir"], tree.display().to_string());
        assert_eq!(parsed["data"]["removed_output_dir"], true);
        assert_eq!(parsed["data"]["deleted_config"], true);
    }

    #[test]
    fn raze_json_reports_suspicious_ledger_entries_as_warnings() {
        let home = TempDir::new().unwrap();
        let tree = seed_tree(&home, "tree");
        bo::domain::index::append_entry(
            &tree.join("index.jsonl"),
            &bo::domain::index::IndexEntry {
                file: "../outside.md".to_string(),
                title: "Suspicious".to_string(),
                url: "https://example.com/suspicious".to_string(),
            },
        )
        .unwrap();

        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "raze", "--json"]);
        assert_eq!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["warnings"][0]["code"], "suspicious_ledger_entry");
        assert_eq!(parsed["warnings"][0]["details"]["file"], "../outside.md");
    }

    #[test]
    fn collect_json_duplicate_url_is_structured_error() {
        let home = TempDir::new().unwrap();
        let tree = seed_tree(&home, "tree");
        let url = "https://www.youtube.com/watch?v=a1mhk7mAetk";
        bo::domain::index::append_entry(
            &tree.join("index.jsonl"),
            &bo::domain::index::IndexEntry {
                file: "existing.md".to_string(),
                title: "Existing Video".to_string(),
                url: url.to_string(),
            },
        )
        .unwrap();

        let (exit, stdout, _stderr) = run_with_home(&home, &["bo", "collect", "--json", url]);
        assert_ne!(exit, 0);
        let parsed = parse_json(&stdout);
        assert_eq!(parsed["ok"], false);
        assert_eq!(parsed["command"], "collect");
        assert_eq!(parsed["error"]["code"], "duplicate_url");
        assert_eq!(parsed["error"]["details"]["existing_file"], "existing.md");
    }

    #[test]
    fn seed_writes_name_derived_from_dir_basename() {
        let home = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();
        let output_dir = output.path().join("my-tree");

        seed_with_home(&home, output_dir, None).unwrap();

        let cfg_path = home.path().join(".bo").join("config.json");
        let cfg = config::read_config(&cfg_path).unwrap();
        assert_eq!(cfg.tree.name.as_deref(), Some("my-tree"));
        assert!(cfg.tree.created_at.is_some());
    }

    #[test]
    fn seed_uses_explicit_name_flag() {
        let home = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();
        let output_dir = output.path().join("some-dir");

        seed_with_home(&home, output_dir, Some("explicit-name".to_string())).unwrap();

        let cfg_path = home.path().join(".bo").join("config.json");
        let cfg = config::read_config(&cfg_path).unwrap();
        assert_eq!(cfg.tree.name.as_deref(), Some("explicit-name"));
    }

    #[test]
    fn seed_already_seeded_is_idempotent() {
        let home = TempDir::new().unwrap();
        let output = TempDir::new().unwrap();
        let output_dir = output.path().join("my-tree");
        let cfg_path = home.path().join(".bo").join("config.json");

        seed_with_home(&home, output_dir.clone(), None).unwrap();
        let first_created_at = config::read_config(&cfg_path).unwrap().tree.created_at;

        seed_with_home(&home, output_dir, None).unwrap();
        let second_created_at = config::read_config(&cfg_path).unwrap().tree.created_at;

        assert_eq!(first_created_at, second_created_at);
    }
}
