use bo::cli::collect::{self, CollectError};
use bo::cli::compile::{self, BranchResult, CompileError, CompileResult};
use bo::cli::json::{self as json_output, JsonError, JsonWarning};
use bo::cli::list::{self, ListOptions};
use bo::cli::query;
use bo::cli::raze;
use bo::cli::search::{self, SearchOptions, SearchQuery};
use bo::cli::seed;
use bo::cli::show::{self, ShowOptions};
use bo::engine::config::{self, Config, ConfigError};
use clap::{error::ErrorKind as ClapErrorKind, Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;

const NOT_SEEDED_MSG: &str = "bo hasn't been seeded yet — run: bo seed <output-dir>";
const KNOWN_COMMANDS: &[&str] = &[
    "seed", "collect", "compile", "list", "search", "show", "query", "raze",
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
    /// Ask a question and get an answer synthesized from collected sources
    Query {
        /// Natural-language question (all arguments joined)
        #[arg(required = true, num_args = 1..)]
        question: Vec<String>,
    },
    /// Delete all bo-managed files and config
    Raze,
}

// ── JSON payloads ────────────────────────────────────────────────────────────

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

// ── errors ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum CliError {
    Usage { message: String, exit_code: i32 },
    NotSeeded,
    ConfigRead(String),
    Seed(seed::SeedError),
    Raze(raze::RazeError),
    Collect(CollectError),
    List(list::ListError),
    Search(search::SearchError),
    Show(show::ShowError),
    Compile(CompileError),
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
            CliError::ConfigRead(message) => JsonError::new("io_error", message.clone()),
            CliError::Seed(error) => JsonError::new("io_error", error.to_string()),
            CliError::Raze(error) => JsonError::new("io_error", error.to_string()),
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
            CliError::ConfigRead(message) => write!(f, "{}", message),
            CliError::Seed(error) => write!(f, "{}", error),
            CliError::Raze(error) => write!(f, "{}", error),
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
        Commands::Seed { output_dir, name } => {
            match seed::seed(output_dir, name, &config::config_path()) {
                Ok(result) if json => emit_json_success("seed", &result, Vec::new(), stdout),
                Ok(result) => {
                    write_human_or_error(write!(stdout, "{}", seed::render_human(&result)), stderr)
                }
                Err(error) => emit_cli_error("seed", json, CliError::Seed(error), stdout, stderr),
            }
        }
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
        Commands::Raze => match require_config() {
            Err(error) => emit_cli_error("raze", json, error, stdout, stderr),
            Ok(cfg) => match raze::raze(&cfg.tree.output_dir, &config::config_path()) {
                Ok(output) if json => {
                    emit_json_success("raze", &output.result, output.warnings, stdout)
                }
                Ok(output) => {
                    for warning in &output.warnings {
                        let _ = writeln!(stderr, "warning: {}", warning.message);
                    }
                    write_human_or_error(
                        write!(stdout, "{}", raze::render_human(&output.result)),
                        stderr,
                    )
                }
                Err(error) => emit_cli_error("raze", json, CliError::Raze(error), stdout, stderr),
            },
        },
        Commands::Query { question } => {
            let question_str = question.join(" ");
            match execute_query(&question_str) {
                Ok(result) if json => emit_json_success("query", &result, Vec::new(), stdout),
                Ok(result) => {
                    write_human_or_error(write!(stdout, "{}", query::render_human(&result)), stderr)
                }
                Err(error) => {
                    let exit_code = error.exit_code();
                    let json_error = query_json_error(&error);
                    if json {
                        emit_json_error("query", json_error, Vec::new(), stdout, exit_code)
                    } else {
                        let _ = writeln!(stderr, "error: {}", error);
                        exit_code
                    }
                }
            }
        }
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

fn execute_collect(url: String) -> Result<CollectResult, CliError> {
    let cfg = require_config()?;
    eprintln!("fetching {}...", url);
    let page = collect::collect_url(&url, &cfg.tree.output_dir).map_err(CliError::Collect)?;
    let path = cfg.tree.output_dir.join(&page.filename);

    Ok(CollectResult {
        url: page.url,
        file: page.filename,
        path: path.display().to_string(),
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

fn execute_query(question: &str) -> Result<query::QueryResult, query::QueryError> {
    let cfg = require_config().map_err(|e| {
        query::QueryError::NoProvider(format!("{}. Cannot query without a configured tree.", e))
    })?;

    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        _ => {
            return Err(query::QueryError::NoProvider(
                "No API key configured. Set OPENAI_API_KEY or configure a provider.".to_string(),
            ))
        }
    };

    let model = cfg.effective_query_model().to_string();
    query::run(&cfg.tree.output_dir, question, &api_key, &model)
}

fn query_json_error(error: &query::QueryError) -> JsonError {
    let code = match error {
        query::QueryError::NoProvider(_) => "no_provider",
        query::QueryError::NoTerms => "no_terms",
        query::QueryError::NoResults => "no_results",
        query::QueryError::EmptyTree => "empty_tree",
        query::QueryError::Io(_) => "io_error",
        query::QueryError::Llm(_) => "llm_error",
        query::QueryError::Parse(_) => "parse_error",
    };
    JsonError::new(code, error.to_string())
}

// ── human rendering ──────────────────────────────────────────────────────────

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
#[path = "tests/main_tests.rs"]
mod tests;
