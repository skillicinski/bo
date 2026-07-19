use bo::cli::collect::{self, BatchCollectResult, CollectError, CollectOutput};
use bo::cli::config::{self as cli_config, ConfigWriteError};
use bo::cli::journal;
use bo::cli::json::{self as json_output, JsonError, JsonWarning};
use bo::cli::list::{self, ListError};
use bo::cli::query;
use bo::cli::raze::{self, RazeError};
use bo::cli::seed::{self, SeedError};
use bo::cli::show::{self, ShowError};
use bo::cli::status::{self, StatusError};
use bo::cli::synthesize::{self, SynthesisError, SynthesisOptions};
use bo::engine::auth;
use bo::engine::config::{self, Config, ConfigError, SeededConfig};
use bo::engine::llm::{self as llm_mod, LlmProvider};
use bo::engine::transaction::TransactionError;
use clap::{error::ErrorKind as ClapErrorKind, Parser, Subcommand};
use serde::Serialize;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;

// main.rs owns all process-shell responsibilities per docs/architecture.md:
// clap argument schema, CliError + exit-code policy, stdout/stderr/JSON
// emission, clap parse-error routing, dependency composition, and the command
// dispatch table. Command policy lives in the cli modules.

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
    /// Collect URLs, a URL list, or local markdown notes
    #[command(
        after_help = "Examples:\n  bo collect https://example.com/a https://example.com/b\n  bo collect urls.txt\n  bo collect ./note.md\n\nURL list files must use .txt and contain one URL per non-empty line.\nLocal .md files are collected as notes (frontmatter stripped, no fetch)."
    )]
    Collect {
        /// URL(s) or a .txt file containing URLs, one per non-empty line
        #[arg(required = true, value_name = "URL_OR_URLS_FILE", num_args = 1..)]
        inputs: Vec<String>,
    },
    /// Configure bo settings (provider, model, compile_model, base_url)
    Config {
        /// LLM provider (openai, deepseek, google, zai, or custom)
        #[arg(long)]
        provider: Option<String>,
        /// Model for LLM operations
        #[arg(long)]
        model: Option<String>,
        /// Model for compile operations (falls back to --model)
        #[arg(long)]
        compile_model: Option<String>,
        /// OpenAI-compatible endpoint prefix for the custom provider
        /// (everything before /chat/completions)
        #[arg(long)]
        base_url: Option<String>,
    },
    /// Compile collected documents into a linked knowledge graph
    Compile {
        /// Recompile the full corpus from scratch — produces a new branch
        /// organization each run
        #[arg(long)]
        all: bool,
        /// Use the iterative agent loop (requires --dry-run in this milestone)
        #[arg(long, requires = "dry_run")]
        agent: bool,
        /// Show a validated preview and write nothing to the tree
        #[arg(long)]
        dry_run: bool,
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
    /// Read the tree's operation journal (append-only log of collects, compiles, queries)
    Journal {
        /// Maximum number of recent events to show (newest last)
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

// ── process-shell types ──────────────────────────────────────────────────────

const NOT_SEEDED_MSG: &str = "bo hasn't been seeded yet — run: bo seed --path <path>";

#[derive(Debug)]
enum CliError {
    NotSeeded,
    ConfigRead(String),
    Seed(SeedError),
    Raze(RazeError),
    Collect(CollectError),
    List(ListError),
    Show(ShowError),
    Compile(SynthesisError),
    Status(StatusError),
    ConfigWrite(ConfigWriteError),
}

impl CliError {
    fn exit_code(&self) -> i32 {
        match self {
            CliError::Seed(error) => error.exit_code(),
            CliError::ConfigWrite(error) => error.exit_code(),
            CliError::Collect(CollectError::Transaction(TransactionError::Busy { .. }))
            | CliError::Compile(SynthesisError::Busy(_))
            | CliError::Raze(RazeError::Busy(_)) => 2,
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
            CliError::Status(error) => match error {
                StatusError::Io(msg) => JsonError::new("io_error", msg),
                StatusError::TreeState(e) => JsonError::new("state_error", e.to_string()),
            },
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

// ── I/O helpers ──────────────────────────────────────────────────────────────

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
            JsonError::with_details(
                "batch_failed",
                result.failure_message(),
                serde_json::json!(result),
            ),
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

// ── argument helpers ─────────────────────────────────────────────────────────

const KNOWN_COMMANDS: &[&str] = &[
    "seed", "config", "collect", "compile", "journal", "list", "show", "query", "status", "raze",
];

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
            serde_json::json!({
                "kind": format!("{:?}", error.kind()),
                "exit_code": exit_code,
            }),
        );
        return emit_json_error(command, json_error, Vec::new(), exit_code);
    }

    wrote(write!(stderr, "{}", error.render()), exit_code)
}

// ── host helpers ─────────────────────────────────────────────────────────────

fn read_config() -> Result<Option<Config>, ConfigError> {
    match config::read_config(&config::config_path()) {
        Ok(cfg) => Ok(Some(cfg)),
        Err(ConfigError::NotFound) => Ok(None),
        Err(e) => Err(e),
    }
}

fn load_config() -> Result<Option<Config>, CliError> {
    read_config().map_err(|e| CliError::ConfigRead(format!("failed to read config: {}", e)))
}

fn require_seeded_config() -> Result<SeededConfig, CliError> {
    let Some(cfg) = load_config()? else {
        return Err(CliError::NotSeeded);
    };
    cfg.into_seeded().ok_or(CliError::NotSeeded)
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
                    CliError::Seed(SeedError::UnsupportedFlag { flag: "--json" }),
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
            base_url,
        } => {
            let provider_opt = match provider {
                Some(ref p) => Some(match bo::engine::llm::Provider::parse(p) {
                    Some(provider) => provider,
                    None => {
                        let err = ConfigWriteError::UnknownProvider { raw: p.clone() };
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
                    base_url,
                },
                &config::config_path(),
            ) {
                Ok(result) if json => emit_json_success("config", &result, Vec::new(), stdout),
                Ok(result) => wrote(write!(stdout, "{}", cli_config::render_human(&result)), 0),
                Err(error) => emit_cli_error("config", json, CliError::ConfigWrite(error), stderr),
            }
        }
        Commands::Collect { inputs } => {
            let mut warnings = Vec::new();
            let cfg = match require_seeded_config() {
                Ok(c) => c,
                Err(error) => return emit_cli_error("collect", json, error, stderr),
            };
            let tree = cfg.tree();
            let model = match cfg.config.effective_model() {
                Ok(m) => m,
                Err(e) => {
                    return emit_cli_error(
                        "collect",
                        json,
                        CliError::ConfigRead(e.to_string()),
                        stderr,
                    );
                }
            };
            let outcome = collect::collect(
                inputs,
                tree.path(),
                cfg.config.provider,
                model.as_str(),
                cfg.config.base_url.as_deref(),
                &mut warnings,
            )
            .map_err(CliError::Collect);

            for line in &warnings {
                let _ = writeln!(stderr, "{}", line);
            }
            match outcome {
                Ok(CollectOutput::Single(result)) if json => {
                    emit_json_success("collect", &result, Vec::new(), stdout)
                }
                Ok(CollectOutput::Single(result)) => {
                    wrote(collect::render_human(&result, stdout), 0)
                }
                Ok(CollectOutput::Batch(result)) if json => {
                    emit_batch_collect_json(&result, stdout)
                }
                Ok(CollectOutput::Batch(result)) => {
                    let exit_code = if result.has_failures() { 1 } else { 0 };
                    match collect::render_batch_human(&result, stdout) {
                        Ok(()) => exit_code,
                        Err(_) => 1,
                    }
                }
                Err(error) => emit_cli_error("collect", json, error, stderr),
            }
        }
        Commands::Compile {
            all,
            agent,
            dry_run,
        } => {
            let cfg = match require_seeded_config() {
                Ok(c) => c,
                Err(error) => return emit_cli_error("compile", json, error, stderr),
            };
            match synthesize::run(
                &cfg,
                SynthesisOptions {
                    all,
                    agent,
                    dry_run,
                },
            ) {
                synthesize::Dispatch::DryRun(outcome) => {
                    let _ = synthesize::render_diagnostics(outcome.stderr_lines(), stderr);
                    match outcome.result {
                        Ok(preview) if json => emit_json_success(
                            "compile",
                            &preview,
                            synthesize::preview_warnings(&preview),
                            stdout,
                        ),
                        Ok(preview) => wrote(
                            synthesize::render_preview_human(&preview, stdout, &cfg.tree().name),
                            0,
                        ),
                        Err(error) => {
                            emit_cli_error("compile", json, CliError::Compile(error), stderr)
                        }
                    }
                }
                synthesize::Dispatch::Live(outcome) => {
                    let _ = synthesize::render_diagnostics(outcome.stderr_lines(), stderr);
                    match outcome.result {
                        Ok(result) if json => emit_json_success(
                            "compile",
                            &result,
                            synthesize::result_warnings(&result),
                            stdout,
                        ),
                        Ok(result) => wrote(
                            synthesize::render_human(&result, stdout, &cfg.tree().name),
                            0,
                        ),
                        Err(error) => {
                            emit_cli_error("compile", json, CliError::Compile(error), stderr)
                        }
                    }
                }
            }
        }
        Commands::List {
            branches,
            leaves,
            terms,
            limit,
            recent,
            branch,
        } => {
            let cfg = match require_seeded_config() {
                Ok(c) => c,
                Err(error) => return emit_cli_error("list", json, error, stderr),
            };
            match list::run(&cfg, branches, leaves, terms, limit, recent, branch) {
                Ok(result) if json => {
                    let warnings = list::warnings(&result);
                    emit_json_success("list", &result, warnings, stdout)
                }
                Ok(result) => wrote(write!(stdout, "{}", list::render_human(&result)), 0),
                Err(error) => emit_cli_error("list", json, CliError::List(error), stderr),
            }
        }
        Commands::Show { title, full } => {
            let cfg = match require_seeded_config() {
                Ok(c) => c,
                Err(error) => return emit_cli_error("show", json, error, stderr),
            };
            match show::run(&cfg, &title, full) {
                Ok(result) if json => emit_json_success("show", &result, Vec::new(), stdout),
                Ok(result) => wrote(write!(stdout, "{}", show::render_human(&result)), 0),
                Err(error) => emit_cli_error("show", json, CliError::Show(error), stderr),
            }
        }
        Commands::Raze { include_auth } => {
            let config_path = config::config_path();
            let auth_path = auth::auth_path();
            let config = match load_config() {
                Ok(c) => c,
                Err(error) => return emit_cli_error("raze", json, error, stderr),
            };
            match raze::run(config, &config_path, &auth_path, include_auth) {
                Ok(Some(output)) if json => {
                    emit_json_success("raze", &output.result, output.warnings, stdout)
                }
                Ok(Some(output)) => {
                    for warning in &output.warnings {
                        let _ = writeln!(stderr, "warning: {}", warning.message);
                    }
                    wrote(write!(stdout, "{}", raze::render_human(&output.result)), 0)
                }
                Ok(None) => emit_cli_error("raze", json, CliError::NotSeeded, stderr),
                Err(error) => emit_cli_error("raze", json, CliError::Raze(error), stderr),
            }
        }
        Commands::Query { question } => {
            let question_str = question.join(" ");
            let resolve_provider =
                |cfg: &SeededConfig| -> Result<Box<dyn LlmProvider>, query::QueryError> {
                    let api_key = auth::resolve_api_key(cfg.config.provider)
                        .map_err(|e| query::QueryError::NoProvider(e.to_string()))?;
                    llm_mod::create_provider(
                        cfg.config.provider,
                        &api_key,
                        cfg.config.base_url.as_deref(),
                    )
                    .map_err(|e| query::QueryError::NoProvider(e.to_string()))
                };
            match query::run(read_config(), &question_str, resolve_provider) {
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
        Commands::Status => {
            let config = match load_config() {
                Ok(c) => c,
                Err(error) => return emit_cli_error("status", json, error, stderr),
            };
            match status::run(config) {
                Ok(result) if json => emit_json_success("status", &result, Vec::new(), stdout),
                Ok(result) => wrote(write!(stdout, "{}", status::render_human(&result)), 0),
                Err(error) => emit_cli_error("status", json, CliError::Status(error), stderr),
            }
        }
        Commands::Journal { limit } => {
            let cfg = match require_seeded_config() {
                Ok(c) => c,
                Err(error) => return emit_cli_error("journal", json, error, stderr),
            };
            let result = journal::run(&cfg, limit);
            if json {
                emit_json_success("journal", &result, Vec::new(), stdout)
            } else {
                wrote(write!(stdout, "{}", journal::render_human(&result)), 0)
            }
        }
    }
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
        .with_writer(io::stderr)
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
