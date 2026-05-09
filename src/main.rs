use bo::cli::collect;
use bo::cli::list::{self, ListOptions};
use bo::cli::show::{self, ShowOptions};
use bo::domain::index;
use bo::engine::config::{self, Config, ConfigError};

use chrono::Utc;
use clap::{Parser, Subcommand};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process;

const NOT_SEEDED_MSG: &str = "bo hasn't been seeded yet — run: bo seed <output-dir>";

#[derive(Parser)]
#[command(name = "bo", about = "Collect web pages into a local markdown tree")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
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
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Show one collected leaf by exact title
    Show {
        /// Leaf title to show
        title: String,
        /// Show the full leaf body instead of a preview
        #[arg(long)]
        full: bool,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Delete all bo-managed files and config
    Raze,
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn not_seeded_err() -> String {
    NOT_SEEDED_MSG.to_string()
}

fn require_config() -> Result<Config, String> {
    match config::read_config(&config::config_path()) {
        Ok(cfg) => Ok(cfg),
        Err(ConfigError::NotFound) => Err(not_seeded_err()),
        Err(e) => Err(format!("failed to read config: {}", e)),
    }
}

// ── cmd_seed ─────────────────────────────────────────────────────────────────

fn cmd_seed(output_dir: PathBuf, name: Option<String>) -> Result<(), String> {
    // Resolve to absolute path
    let output_dir = if output_dir.is_absolute() {
        output_dir
    } else {
        std::env::current_dir()
            .map_err(|e| format!("failed to get current dir: {}", e))?
            .join(&output_dir)
    };

    // Check if already seeded
    match config::read_config(&config::config_path()) {
        Ok(existing) => {
            println!(
                "bo has already been seeded at {}!",
                existing.tree.output_dir.display()
            );
            return Ok(());
        }
        Err(ConfigError::NotFound) => {} // proceed
        Err(e) => return Err(format!("failed to read config: {}", e)),
    }

    std::fs::create_dir_all(&output_dir)
        .map_err(|e| format!("failed to create output directory: {}", e))?;

    // Derive tree name from dir basename when --name is not provided
    let tree_name = name.or_else(|| {
        output_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    });

    let created_at = Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    config::write_config(
        &Config {
            tree: bo::domain::tree::TreeConfig {
                output_dir: output_dir.clone(),
                name: tree_name,
                created_at: Some(created_at),
            },
            compile_model: None,
        },
        &config::config_path(),
    )
    .map_err(|e| format!("failed to write config: {}", e))?;

    println!("seeded bo at {}", output_dir.display());
    Ok(())
}

// ── cmd_add ──────────────────────────────────────────────────────────────────

fn cmd_collect(url: String) -> Result<(), String> {
    let cfg = require_config()?;
    eprintln!("fetching {}...", url);
    let page = collect::collect_url(&url, &cfg.tree.output_dir).map_err(|e| e.to_string())?;
    println!("✓ collected: {} → {}", page.url, page.filename);
    Ok(())
}

// ── cmd_list ─────────────────────────────────────────────────────────────────

fn cmd_list(
    limit: Option<usize>,
    recent: bool,
    branch: Option<String>,
    json: bool,
) -> Result<(), String> {
    let cfg = require_config()?;
    let result = list::list_leaves(
        &cfg.tree.output_dir,
        &ListOptions {
            limit,
            recent,
            branch,
        },
    )
    .map_err(|e| e.to_string())?;

    let output = if json {
        list::render_json(&result).map_err(|e| e.to_string())?
    } else {
        list::render_human(&result)
    };

    print!("{output}");
    Ok(())
}

// ── cmd_show ─────────────────────────────────────────────────────────────────

fn cmd_show(title: String, full: bool, json: bool) -> Result<(), String> {
    let cfg = require_config()?;
    let result = show::show_leaf(&cfg.tree.output_dir, &title, &ShowOptions { full })
        .map_err(|e| e.to_string())?;

    let output = if json {
        show::render_json(&result).map_err(|e| e.to_string())?
    } else {
        show::render_human(&result)
    };

    print!("{output}");
    Ok(())
}

// ── cmd_raze ─────────────────────────────────────────────────────────────────

fn cmd_raze() -> Result<(), String> {
    let cfg = require_config()?;
    let output_dir = cfg.tree.output_dir;

    let index_path = output_dir.join("index.jsonl");
    let entries =
        index::read_index(&index_path).map_err(|e| format!("failed to read index: {}", e))?;

    // Delete ledger-tracked markdown files
    let mut deleted = 0usize;
    for entry in &entries {
        let resolved = output_dir.join(&entry.file);

        // Path traversal guard
        if !resolved.starts_with(&output_dir) {
            eprintln!(
                "warning: skipping ledger entry with suspicious path: {}",
                entry.file
            );
            continue;
        }

        match std::fs::remove_file(&resolved) {
            Ok(()) => deleted += 1,
            Err(e) if e.kind() == ErrorKind::NotFound => {} // already gone, fine
            Err(e) => return Err(format!("failed to delete {}: {}", resolved.display(), e)),
        }
    }
    println!("deleted {} markdown file(s)", deleted);

    // Delete index
    match std::fs::remove_file(&index_path) {
        Ok(()) => println!("deleted index"),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("failed to delete ledger: {}", e)),
    }

    // Attempt to remove output dir (only succeeds if empty)
    match std::fs::remove_dir(&output_dir) {
        Ok(()) => println!("removed output directory {}", output_dir.display()),
        Err(e) if e.kind() == ErrorKind::DirectoryNotEmpty || e.kind() == ErrorKind::NotFound => {
            println!(
                "output directory left in place (not empty or already absent): {}",
                output_dir.display()
            );
        }
        Err(e) => return Err(format!("failed to remove output directory: {}", e)),
    }

    // Delete config last — so a mid-raze failure doesn't strand the user
    match std::fs::remove_file(config::config_path()) {
        Ok(()) => println!("deleted config"),
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => return Err(format!("failed to delete config: {}", e)),
    }

    Ok(())
}

// ── main ─────────────────────────────────────────────────────────────────────

fn main() {
    // Load .env if present (no-op if missing)
    let _ = dotenvy::dotenv();

    // Initialise tracing. WARN+ shown by default; set RUST_LOG=debug for verbose output.
    // Format is message-only (no timestamp/level prefix) to match the CLI's plain output style.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::WARN.into()),
        )
        .with_target(false)
        .without_time()
        .with_level(false)
        .init();

    let cli = Cli::parse();
    let result = match cli.command {
        Commands::Seed { output_dir, name } => cmd_seed(output_dir, name),
        Commands::Collect { url } => cmd_collect(url),
        Commands::Compile => require_config().and_then(|cfg| bo::cli::compile::cmd_compile(&cfg)),
        Commands::List {
            limit,
            recent,
            branch,
            json,
        } => cmd_list(limit, recent, branch, json),
        Commands::Show { title, full, json } => cmd_show(title, full, json),
        Commands::Raze => cmd_raze(),
    };
    if let Err(e) = result {
        eprintln!("error: {}", e);
        process::exit(1);
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use bo::engine::config;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // Tests that mutate the HOME env var must run serially.
    static HOME_LOCK: Mutex<()> = Mutex::new(());

    /// Run cmd_seed with HOME redirected to a temp directory.
    fn seed_with_home(
        home: &TempDir,
        output_dir: PathBuf,
        name: Option<String>,
    ) -> Result<(), String> {
        let _guard = HOME_LOCK.lock().unwrap();
        std::env::set_var("HOME", home.path());
        cmd_seed(output_dir, name)
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

        // First seed
        seed_with_home(&home, output_dir.clone(), None).unwrap();
        let first_created_at = config::read_config(&cfg_path).unwrap().tree.created_at;

        // Second seed — should be a no-op (already seeded message, config unchanged)
        seed_with_home(&home, output_dir, None).unwrap();
        let second_created_at = config::read_config(&cfg_path).unwrap().tree.created_at;

        assert_eq!(first_created_at, second_created_at);
    }
}
