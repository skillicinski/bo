use bo::cli::collect::{self, BatchCollectResult, CollectError, CollectOutput};
use bo::cli::compile::{self, CompileError, CompileOptions, CompileResult};
use bo::cli::config as cli_config;
use bo::cli::json::{self as json_output, JsonError, JsonWarning};
use bo::cli::list::{self};
use bo::cli::query;
use bo::cli::raze;
use bo::cli::seed;
use bo::cli::show::{self, ShowOptions};
use bo::cli::status;
use bo::engine::auth;
use bo::engine::config::{self, ConfigError, SeededConfig};
use bo::engine::llm::{self, LlmProvider, Provider};
use clap::{error::ErrorKind as ClapErrorKind, Parser, Subcommand};
use serde::Serialize;
use serde_json::json;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;

const NOT_SEEDED_MSG: &str = "bo hasn't been seeded yet — run: bo seed --path <path>";
const KNOWN_COMMANDS: &[&str] = &[
    "seed", "config", "collect", "compile", "list", "show", "query", "status", "raze",
];

#[derive(Parser, Debug)]
#[command(
    name = "bo",
    about = "Collect web pages into a local markdown tree",
    version
)]
struct Cli {
    /// Emit machine-readable JSON
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialise a tree and save config
    Seed {
        /// Directory to store collected content
        #[arg(long)]
        path: Option<PathBuf>,
        /// Human-readable name for the tree
        #[arg(long)]
        name: Option<String>,
        /// LLM provider (openai, deepseek, or google)
        #[arg(long)]
        provider: Option<String>,
        /// Model for LLM operations
        #[arg(long)]
        model: Option<String>,
    },
    /// Fetch one or more URLs and collect them
    #[command(
        after_help = "Examples:\n  bo collect https://example.com/a https://example.com/b\n  bo collect urls.txt\n\nURL list files must use .txt and contain one URL per non-empty line."
    )]
    Collect {
        /// URL(s) or a .txt file containing URLs, one per non-empty line
        #[arg(required = true, value_name = "URL_OR_URLS_FILE", num_args = 1..)]
        inputs: Vec<String>,
    },
    /// Configure bo settings (provider, model, compile_model)
    Config {
        /// LLM provider (openai or deepseek)
        #[arg(long)]
        provider: Option<String>,
        /// Model for LLM operations
        #[arg(long)]
        model: Option<String>,
        /// Model for compile operations (falls back to --model)
        #[arg(long)]
        compile_model: Option<String>,
    },
    /// Compile collected documents into a linked knowledge graph
    Compile {
        /// Recompile the full corpus and allow complete branch graph rewrite
        #[arg(long)]
        all: bool,
    },
    /// Inspect branches and leaves in the current tree
    #[command(
        after_help = "Examples:\n  bo list                  # branch-centric tree view\n  bo list --branches       # flat branch list with leaf counts\n  bo list --leaves         # flat leaf list with branch counts\n  bo list --terms rust     # filter by title/slug match"
    )]
    List {
        /// Show only branches (flat list with leaf counts)
        #[arg(long, conflicts_with = "leaves")]
        branches: bool,
        /// Show only leaves (flat list with branch counts)
        #[arg(long, conflicts_with = "branches")]
        leaves: bool,
        /// Filter by text match against title and slug (all terms must match)
        #[arg(long, num_args = 1.., value_name = "TERMS")]
        terms: Vec<String>,
        /// Maximum number of items to show
        #[arg(long)]
        limit: Option<usize>,
        /// Sort leaves by collected date, newest first (only in --leaves mode)
        #[arg(long)]
        recent: bool,
        /// Filter by exact branch name/slug
        #[arg(long)]
        branch: Option<String>,
    },
    /// Display a leaf's frontmatter card (use --full for complete body)
    Show {
        /// Leaf title to show
        title: String,
        /// Include the complete leaf body
        #[arg(long)]
        full: bool,
    },
    /// Ask a question and get an answer synthesized from collected sources
    Query {
        /// Natural-language question (all arguments joined)
        #[arg(required = true, num_args = 1..)]
        question: Vec<String>,
    },
    /// Delete the seeded tree and config
    Raze {
        /// Also delete stored provider credentials (~/.bo/auth.json)
        #[arg(long)]
        include_auth: bool,
    },
    /// Show tree health and compile readiness
    Status,
}

// ── JSON payloads ────────────────────────────────────────────────────────────

// ── errors ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
enum CliError {
    NotSeeded,
    ConfigRead(String),
    Seed(seed::SeedError),
    Raze(raze::RazeError),
    Collect(CollectError),
    List(list::ListError),
    Show(show::ShowError),
    Compile(CompileError),
    Status(status::StatusError),
    ConfigWrite(cli_config::ConfigWriteError),
}

impl CliError {
    fn exit_code(&self) -> i32 {
        match self {
            CliError::Seed(error) => error.exit_code(),
            CliError::ConfigWrite(error) => error.exit_code(),
            CliError::Collect(CollectError::Pending(bo::engine::pending::PendingError::Busy {
                ..
            }))
            | CliError::Compile(CompileError::Busy(_))
            | CliError::Raze(raze::RazeError::Busy(_)) => 2,
            _ => 1,
        }
    }

    fn to_json_error(&self) -> JsonError {
        match self {
            CliError::NotSeeded => JsonError::new("not_seeded", NOT_SEEDED_MSG),
            CliError::ConfigRead(message) => JsonError::new("io_error", message.clone()),
            CliError::Seed(error) => JsonError::new("io_error", error.to_string()),
            CliError::Raze(error) => JsonError::new("io_error", error.to_string()),
            CliError::Collect(error) => error.json_error(),
            CliError::List(error) => error.json_error(),
            CliError::Show(error) => error.json_error(),
            CliError::Compile(error) => error.json_error(),
            CliError::Status(error) => JsonError::new("io_error", error.to_string()),
            CliError::ConfigWrite(error) => {
                JsonError::with_details(error.code(), error.to_string(), error.details())
            }
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::NotSeeded => write!(f, "{}", NOT_SEEDED_MSG),
            CliError::ConfigRead(message) => write!(f, "{}", message),
            CliError::Seed(error) => write!(f, "{}", error),
            CliError::Raze(error) => write!(f, "{}", error),
            CliError::Collect(error) => write!(f, "{}", error),
            CliError::List(error) => write!(f, "{}", error),
            CliError::Show(error) => write!(f, "{}", error),
            CliError::Compile(error) => write!(f, "{}", error),
            CliError::Status(error) => write!(f, "{}", error),
            CliError::ConfigWrite(error) => write!(f, "{}", error),
        }
    }
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
        Commands::Seed {
            path,
            name,
            provider,
            model,
        } => {
            if json {
                return emit_cli_error(
                    "seed",
                    false,
                    CliError::Seed(seed::SeedError::UnsupportedFlag { flag: "--json" }),
                    stderr,
                );
            }

            let mut prompt = seed::StdioSeedPrompt;
            match seed::seed(
                seed::SeedOptions {
                    path,
                    name,
                    provider,
                    model,
                },
                &config::config_path(),
                &mut prompt,
            ) {
                Ok(result) => wrote(write!(stdout, "{}", seed::render_human(&result)), 0),
                Err(error) => emit_cli_error("seed", false, CliError::Seed(error), stderr),
            }
        }
        Commands::Config {
            provider,
            model,
            compile_model,
        } => {
            let provider_opt = match provider {
                Some(ref p) => Some(match Provider::parse(p) {
                    Some(provider) => provider,
                    None => {
                        let err = cli_config::ConfigWriteError::UnknownProvider { raw: p.clone() };
                        return emit_cli_error("config", json, CliError::ConfigWrite(err), stderr);
                    }
                }),
                None => None,
            };

            match cli_config::write_config(
                cli_config::WriteConfigOptions {
                    provider: provider_opt,
                    model,
                    compile_model,
                },
                &config::config_path(),
            ) {
                Ok(result) if json => emit_json_success("config", &result, Vec::new(), stdout),
                Ok(result) => wrote(write!(stdout, "{}", cli_config::render_human(&result)), 0),
                Err(error) => emit_cli_error("config", json, CliError::ConfigWrite(error), stderr),
            }
        }
        Commands::Collect { inputs } => match execute_collect(inputs) {
            Ok(CollectOutput::Single(result)) if json => {
                emit_json_success("collect", &result, Vec::new(), stdout)
            }
            Ok(CollectOutput::Single(result)) => wrote(collect::render_human(&result, stdout), 0),
            Ok(CollectOutput::Batch(result)) if json => emit_batch_collect_json(&result, stdout),
            Ok(CollectOutput::Batch(result)) => {
                let exit_code = if result.has_failures() { 1 } else { 0 };
                match collect::render_batch_human(&result, stdout) {
                    Ok(()) => exit_code,
                    Err(_) => 1,
                }
            }
            Err(error) => emit_cli_error("collect", json, error, stderr),
        },
        Commands::Compile { all } => match require_seeded_config().and_then(|config| {
            compile::run_compile_with_options(&config, CompileOptions { all })
                .map_err(CliError::Compile)
        }) {
            Ok(result) if json => {
                let warnings = compile_warnings(&result);
                emit_json_success("compile", &result, warnings, stdout)
            }
            Ok(result) => {
                let tree_name = require_seeded_config()
                    .ok()
                    .map(|c| c.tree().name)
                    .unwrap_or_else(|| "bo".to_string());
                wrote(compile::render_human(&result, stdout, &tree_name), 0)
            }
            Err(error) => emit_cli_error("compile", json, error, stderr),
        },
        Commands::List {
            branches,
            leaves,
            terms,
            limit,
            recent,
            branch,
        } => match execute_list(branches, leaves, terms, limit, recent, branch) {
            Ok(result) if json => {
                let warnings = list_warnings(&result);
                emit_json_success("list", &result, warnings, stdout)
            }
            Ok(result) => wrote(write!(stdout, "{}", list::render_human(&result)), 0),
            Err(error) => emit_cli_error("list", json, error, stderr),
        },
        Commands::Show { title, full } => match execute_show(title, full) {
            Ok(result) if json => emit_json_success("show", &result, Vec::new(), stdout),
            Ok(result) => wrote(write!(stdout, "{}", show::render_human(&result)), 0),
            Err(error) => emit_cli_error("show", json, error, stderr),
        },
        Commands::Raze { include_auth } => match execute_raze(include_auth) {
            Ok(output) if json => {
                emit_json_success("raze", &output.result, output.warnings, stdout)
            }
            Ok(output) => {
                for warning in &output.warnings {
                    let _ = writeln!(stderr, "warning: {}", warning.message);
                }
                wrote(write!(stdout, "{}", raze::render_human(&output.result)), 0)
            }
            Err(error) => emit_cli_error("raze", json, error, stderr),
        },
        Commands::Query { question } => {
            let question_str = question.join(" ");
            match execute_query(&question_str) {
                Ok(result) if json => emit_json_success("query", &result, Vec::new(), stdout),
                Ok(result) => wrote(write!(stdout, "{}", query::render_human(&result)), 0),
                Err(error) => {
                    let exit_code = error.exit_code();
                    let json_error = error.json_error();
                    if json {
                        emit_json_error("query", json_error, Vec::new(), exit_code)
                    } else {
                        query::render_error_human(&error, stderr, exit_code)
                    }
                }
            }
        }
        Commands::Status => match execute_status() {
            Ok(result) if json => emit_json_success("status", &result, Vec::new(), stdout),
            Ok(result) => wrote(write!(stdout, "{}", status::render_human(&result)), 0),
            Err(error) => emit_cli_error("status", json, error, stderr),
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
        return wrote(write!(stdout, "{}", error.render()), exit_code);
    }

    let command = infer_command(args);
    if raw_json_mode && command == "seed" {
        return wrote(write!(stderr, "{}", error.render()), exit_code);
    }

    if raw_json_mode {
        let rendered = error.render().to_string();
        let json_error = JsonError::with_details(
            "usage_error",
            rendered.trim().to_string(),
            json!({
                "kind": format!("{:?}", error.kind()),
                "exit_code": exit_code,
            }),
        );
        return emit_json_error(command, json_error, Vec::new(), exit_code);
    }

    wrote(write!(stderr, "{}", error.render()), exit_code)
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

fn wrote(result: io::Result<()>, exit_code: i32) -> i32 {
    match result {
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
            1,
        ),
    }
}

fn emit_batch_collect_json<W: Write>(result: &BatchCollectResult, stdout: &mut W) -> i32 {
    if result.has_failures() {
        return emit_json_error(
            "collect",
            JsonError::with_details("batch_failed", result.failure_message(), json!(result)),
            Vec::new(),
            1,
        );
    }

    emit_json_success("collect", result, Vec::new(), stdout)
}

fn emit_json_error(
    command: &str,
    error: JsonError,
    warnings: Vec<JsonWarning>,
    exit_code: i32,
) -> i32 {
    let mut stderr = io::stderr();
    match json_output::error_string(command, error, warnings) {
        Ok(encoded) => match writeln!(stderr, "{}", encoded) {
            Ok(()) => exit_code,
            Err(_) => 1,
        },
        Err(_) => 1,
    }
}

fn emit_cli_error<E: Write>(command: &str, json: bool, error: CliError, stderr: &mut E) -> i32 {
    let exit_code = error.exit_code();
    if json {
        return emit_json_error(command, error.to_json_error(), Vec::new(), exit_code);
    }

    match writeln!(stderr, "error: {}", error) {
        Ok(()) => exit_code,
        Err(_) => 1,
    }
}

// ── command execution ────────────────────────────────────────────────────────

fn require_seeded_config() -> Result<SeededConfig, CliError> {
    let seeded = match config::read_config(&config::config_path()) {
        Ok(cfg) => cfg.into_seeded().ok_or(CliError::NotSeeded),
        Err(ConfigError::NotFound) => Err(CliError::NotSeeded),
        Err(error) => Err(CliError::ConfigRead(format!(
            "failed to read config: {}",
            error
        ))),
    }?;
    Ok(seeded)
}

fn execute_status() -> Result<status::StatusResult, CliError> {
    let config = match config::read_config(&config::config_path()) {
        Ok(c) => Some(c),
        Err(ConfigError::NotFound) => None,
        Err(e) => {
            return Err(CliError::ConfigRead(format!(
                "failed to read config: {}",
                e
            )))
        }
    };

    let Some(tree) = config
        .clone()
        .and_then(config::Config::into_seeded)
        .map(|c| c.tree())
    else {
        return Ok(status::config_only_status(config.as_ref()));
    };

    status::compute_status(tree.path(), &tree.name, config.as_ref()).map_err(CliError::Status)
}

fn execute_raze(include_auth: bool) -> Result<raze::RazeOutput, CliError> {
    let config_path = config::config_path();
    let auth_path = auth::auth_path();

    match config::read_config(&config_path) {
        Ok(cfg) => {
            let seeded = cfg.into_seeded().ok_or(CliError::NotSeeded)?;
            let auth_cleanup = if include_auth {
                raze::AuthCleanup::Delete
            } else {
                raze::AuthCleanup::Preserve
            };
            let tree = seeded.tree();
            raze::raze_with_auth(tree.path(), &config_path, &auth_path, auth_cleanup)
                .map_err(CliError::Raze)
        }
        Err(ConfigError::NotFound) => {
            if !include_auth {
                return Err(CliError::NotSeeded);
            }

            match raze::raze_auth_only(&auth_path).map_err(CliError::Raze)? {
                Some(output) => Ok(output),
                None => Ok(raze::RazeOutput {
                    result: raze::RazeResult {
                        auth_path: auth_path.to_string_lossy().into_owned(),
                        ..Default::default()
                    },
                    warnings: Vec::new(),
                }),
            }
        }
        Err(error) => Err(CliError::ConfigRead(format!(
            "failed to read config: {}",
            error
        ))),
    }
}

fn execute_collect(inputs: Vec<String>) -> Result<CollectOutput, CliError> {
    let cfg = require_seeded_config()?;
    let tree = cfg.tree();
    let output_dir = tree.path().to_path_buf();
    let model = cfg
        .config
        .effective_model()
        .map_err(|e| CliError::ConfigRead(e.to_string()))?;

    // Use parallel path for multiple URLs or file-based input.
    let use_parallel = inputs.len() > 1
        || inputs
            .iter()
            .any(|i| i.ends_with(".txt") && !i.contains("://"));

    if use_parallel {
        let result = collect::collect_batch_parallel(
            inputs,
            &output_dir,
            model.as_str(),
            cfg.config.provider,
        )
        .map_err(CliError::Collect)?;
        return Ok(CollectOutput::Batch(result));
    }

    // ponytail: single URL — call collect_url_with_model directly.
    // collect_inputs_with_collector kept for unit tests only.
    let url = &inputs[0];
    eprintln!("fetching {}...", url);
    let doc =
        collect::collect_url_with_model(url, &output_dir, model.as_str(), cfg.config.provider)
            .map_err(CliError::Collect)?;
    let path = output_dir.join(&doc.filename);
    Ok(CollectOutput::Single(collect::CollectResult {
        url: doc.url,
        file: doc.filename,
        path: path.display().to_string(),
    }))
}

fn execute_list(
    branches: bool,
    leaves: bool,
    terms: Vec<String>,
    limit: Option<usize>,
    recent: bool,
    branch: Option<String>,
) -> Result<list::ListResult, CliError> {
    let view = if leaves {
        list::ListViewMode::Leaves
    } else if branches {
        list::ListViewMode::Branches
    } else {
        list::ListViewMode::BranchCentric
    };
    let cfg = require_seeded_config()?;
    let tree = cfg.tree();
    list::list_tree(
        tree.path(),
        &list::ListOptions {
            view,
            terms: terms.iter().map(|t| t.to_lowercase()).collect(),
            limit,
            recent,
            branch,
        },
    )
    .map_err(CliError::List)
}

fn execute_show(title: String, full: bool) -> Result<show::ShowResult, CliError> {
    let cfg = require_seeded_config()?;
    let tree = cfg.tree();
    show::show_leaf(tree.path(), &title, &ShowOptions { full }).map_err(CliError::Show)
}

fn execute_query(question: &str) -> Result<query::QueryResult, query::QueryError> {
    let cfg = require_seeded_config().map_err(|e| {
        query::QueryError::NoProvider(format!("{}. Cannot query without a configured tree.", e))
    })?;

    execute_query_with_provider_resolver(&cfg, question, || {
        let api_key = auth::resolve_api_key(cfg.config.provider)
            .map_err(|e| query::QueryError::NoProvider(e.to_string()))?;
        Ok(llm::create_provider(cfg.config.provider, &api_key))
    })
}

fn execute_query_with_provider_resolver<F>(
    cfg: &SeededConfig,
    question: &str,
    resolve_provider: F,
) -> Result<query::QueryResult, query::QueryError>
where
    F: FnOnce() -> Result<Box<dyn LlmProvider>, query::QueryError>,
{
    let model = cfg
        .config
        .effective_model()
        .map_err(|e| query::QueryError::NoProvider(e.to_string()))?;
    let tree = cfg.tree();
    let prepared = query::prepare(tree.path(), question, &model)?;
    let provider = resolve_provider()?;
    query::run_prepared_with_provider(prepared, provider.as_ref())
}

fn list_warnings(result: &list::ListResult) -> Vec<JsonWarning> {
    result
        .degraded_leaves()
        .iter()
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
    let mut warnings = Vec::new();

    if !result.leaves_skipped.is_empty() {
        warnings.push(JsonWarning::with_details(
            "skipped_leaves",
            format!(
                "skipped {} leaves with unparseable frontmatter",
                result.leaves_skipped.len()
            ),
            json!({ "files": result.leaves_skipped }),
        ));
    }

    if let Some(msg) =
        compile::degenerate_result_warning(result.mode, &result.branches, result.leaves_processed)
    {
        warnings.push(JsonWarning::new("degenerate_result", msg));
    }

    warnings
}

// ── main ─────────────────────────────────────────────────────────────────────

fn main() {
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
