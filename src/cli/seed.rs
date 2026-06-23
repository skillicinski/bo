use crate::domain::tree::TreeConfig;
use crate::domain::Timestamp;
use crate::engine::config::{self, Config, ConfigError};
use crate::engine::llm::models::{is_supported_model, models_for};
use crate::engine::llm::{Provider, ALL_PROVIDERS};

use std::ffi::OsStr;
use std::fmt;
use std::io::{self, IsTerminal, Write};
use std::path::{Component, Path, PathBuf};

const DEFAULT_TREE_NAME: &str = "bo";

#[derive(Debug, Clone)]
pub struct SeedOptions {
    pub path: Option<PathBuf>,
    pub name: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedStatus {
    Created,
    AlreadyExists,
}

#[derive(Debug, Clone)]
pub struct SeedResult {
    pub status: SeedStatus,
    pub path: PathBuf,
    pub name: String,
    pub created_at: Timestamp,
    pub provider: Provider,
    pub model: String,
    pub compile_model: Option<String>,
}

#[derive(Debug)]
pub enum SeedError {
    UnsupportedFlag {
        flag: &'static str,
    },
    MissingInput {
        field: &'static str,
        flag: &'static str,
    },
    UnknownProvider {
        raw: String,
    },
    UnsupportedModel {
        model: String,
        provider: Provider,
    },
    TreeAlreadySeeded {
        existing_path: PathBuf,
        requested_path: PathBuf,
    },
    ConfigRead(String),
    ConfigWrite(String),
    CreateTreeDir(String),
    CurrentDir(String),
    PromptIo(String),
}

impl SeedError {
    pub fn exit_code(&self) -> i32 {
        match self {
            SeedError::UnsupportedFlag { .. }
            | SeedError::MissingInput { .. }
            | SeedError::UnknownProvider { .. }
            | SeedError::UnsupportedModel { .. }
            | SeedError::TreeAlreadySeeded { .. } => 2,
            SeedError::ConfigRead(_)
            | SeedError::ConfigWrite(_)
            | SeedError::CreateTreeDir(_)
            | SeedError::CurrentDir(_)
            | SeedError::PromptIo(_) => 1,
        }
    }
}

impl fmt::Display for SeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SeedError::UnsupportedFlag { flag } => {
                write!(f, "the `{flag}` flag is not supported for seed.")
            }
            SeedError::MissingInput { field, flag } => write!(
                f,
                "missing seed {field}; rerun with `{flag}` or run `bo seed` in a terminal"
            ),
            SeedError::UnknownProvider { raw } => write!(
                f,
                "unknown provider '{}'; valid providers: {}",
                raw,
                ALL_PROVIDERS.join(", ")
            ),
            SeedError::UnsupportedModel { model, provider } => {
                let supported = models_for(*provider)
                    .iter()
                    .map(|model| model.id)
                    .collect::<Vec<_>>()
                    .join(", ");
                write!(
                    f,
                    "unsupported model '{model}' for provider '{provider}'; supported models: {supported}"
                )
            }
            SeedError::TreeAlreadySeeded {
                existing_path,
                requested_path,
            } => write!(
                f,
                "bo is single-tree right now; already seeded at {}, refusing to seed {}",
                existing_path.display(),
                requested_path.display()
            ),
            SeedError::ConfigRead(message)
            | SeedError::ConfigWrite(message)
            | SeedError::CreateTreeDir(message)
            | SeedError::CurrentDir(message)
            | SeedError::PromptIo(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for SeedError {}

pub trait SeedPrompt {
    fn prompt_path(&mut self) -> Result<PathBuf, SeedError>;
    fn prompt_name(&mut self, default: &str) -> Result<String, SeedError>;
    fn prompt_provider(&mut self) -> Result<Provider, SeedError>;
    fn prompt_model(&mut self, provider: Provider) -> Result<String, SeedError>;
}

pub struct StdioSeedPrompt;

impl SeedPrompt for StdioSeedPrompt {
    fn prompt_path(&mut self) -> Result<PathBuf, SeedError> {
        require_terminal("path", "--path <path>")?;
        loop {
            let input = read_prompt("Tree path: ")?;
            if !input.is_empty() {
                return Ok(PathBuf::from(input));
            }
            eprintln!("path is required");
        }
    }

    fn prompt_name(&mut self, default: &str) -> Result<String, SeedError> {
        require_terminal("name", "--name <name>")?;
        let input = read_prompt(&format!("Tree name [{default}]: "))?;
        Ok(non_empty_or_default(input, default))
    }

    fn prompt_provider(&mut self) -> Result<Provider, SeedError> {
        require_terminal("provider", "--provider <provider>")?;
        loop {
            let input = read_prompt(&format!("Provider ({}): ", ALL_PROVIDERS.join(", ")))?;
            if let Some(provider) = Provider::parse(&input) {
                return Ok(provider);
            }
            eprintln!(
                "unknown provider; valid providers: {}",
                ALL_PROVIDERS.join(", ")
            );
        }
    }

    fn prompt_model(&mut self, provider: Provider) -> Result<String, SeedError> {
        require_terminal("model", "--model <model>")?;
        let supported = supported_models(provider).join(", ");
        loop {
            let input = read_prompt(&format!("Model for {provider} ({supported}): "))?;
            let model = input.trim().to_string();
            if is_supported_model(provider, &model) {
                return Ok(model);
            }
            eprintln!(
                "unsupported model for {provider}; supported models: {}",
                supported
            );
        }
    }
}

pub fn seed(
    options: SeedOptions,
    global_config_path: &Path,
    prompt: &mut dyn SeedPrompt,
) -> Result<SeedResult, SeedError> {
    if let Some(existing) = read_existing_seed(global_config_path)? {
        return existing_seed_result(existing, options.path.as_deref());
    }

    let path = resolve_fresh_path(options.path, prompt)?;
    let name = resolve_name(options.name, prompt)?;
    let provider = resolve_provider(options.provider, prompt)?;
    let model = resolve_model(options.model, provider, prompt)?;
    let created_at = Timestamp::now();

    config::write_config(
        &Config {
            provider,
            model: Some(model.clone()),
            compile_model: None,
            tree: Some(TreeConfig {
                path: path.clone(),
                name: name.clone(),
                created_at: created_at.clone(),
            }),
        },
        global_config_path,
    )
    .map_err(|error| SeedError::ConfigWrite(format!("failed to write config: {error}")))?;

    Ok(SeedResult {
        status: SeedStatus::Created,
        path,
        name,
        created_at,
        provider,
        model,
        compile_model: None,
    })
}

pub fn render_human(result: &SeedResult) -> String {
    let heading = match result.status {
        SeedStatus::Created => "seeded bo",
        SeedStatus::AlreadyExists => "bo is already seeded",
    };
    let mut lines = vec![
        heading.to_string(),
        "tree:".to_string(),
        format!("  path: {}", result.path.display()),
        format!("  name: {}", result.name),
        format!("  created_at: {}", result.created_at),
        format!("provider: {}", result.provider),
        format!("model: {}", result.model),
    ];

    if let Some(compile_model) = &result.compile_model {
        lines.push(format!("compile_model: {compile_model}"));
    }

    if result.status == SeedStatus::AlreadyExists {
        lines.push("use `bo config` to change provider or model".to_string());
    }

    lines.join("\n") + "\n"
}

#[derive(Debug, Clone)]
struct ExistingSeed {
    path: PathBuf,
    name: String,
    created_at: Timestamp,
    provider: Provider,
    model: String,
    compile_model: Option<String>,
}

fn read_existing_seed(global_config_path: &Path) -> Result<Option<ExistingSeed>, SeedError> {
    match config::read_config(global_config_path) {
        Ok(config) => Ok(valid_existing_seed(config)),
        Err(ConfigError::NotFound) | Err(ConfigError::Parse(_)) => Ok(None),
        Err(error) => Err(SeedError::ConfigRead(format!(
            "failed to read config: {error}"
        ))),
    }
}

fn valid_existing_seed(config: Config) -> Option<ExistingSeed> {
    let tree = config.tree?;
    let model = config.model?;
    if !is_supported_model(config.provider, &model) {
        return None;
    }
    if let Some(compile_model) = config.compile_model.as_ref() {
        if !is_supported_model(config.provider, compile_model) {
            return None;
        }
    }

    if !tree.path.is_dir() {
        return None;
    }

    Some(ExistingSeed {
        path: comparable_path(&tree.path).ok()?,
        name: tree.name,
        created_at: tree.created_at,
        provider: config.provider,
        model,
        compile_model: config.compile_model,
    })
}

fn existing_seed_result(
    existing: ExistingSeed,
    requested_path: Option<&Path>,
) -> Result<SeedResult, SeedError> {
    if let Some(requested_path) = requested_path {
        let requested_path = comparable_path(requested_path)?;
        if requested_path != existing.path {
            return Err(SeedError::TreeAlreadySeeded {
                existing_path: existing.path,
                requested_path,
            });
        }
    }

    Ok(SeedResult {
        status: SeedStatus::AlreadyExists,
        path: existing.path,
        name: existing.name,
        created_at: existing.created_at,
        provider: existing.provider,
        model: existing.model,
        compile_model: existing.compile_model,
    })
}

fn resolve_fresh_path(
    path: Option<PathBuf>,
    prompt: &mut dyn SeedPrompt,
) -> Result<PathBuf, SeedError> {
    let path = match path {
        Some(path) if path.as_os_str() != OsStr::new("") => path,
        _ => prompt.prompt_path()?,
    };
    canonical_seed_path(&path)
}

fn resolve_name(name: Option<String>, prompt: &mut dyn SeedPrompt) -> Result<String, SeedError> {
    match name {
        Some(name) => Ok(non_empty_or_default(name, DEFAULT_TREE_NAME)),
        None => match prompt.prompt_name(DEFAULT_TREE_NAME) {
            Ok(name) => Ok(name),
            Err(SeedError::MissingInput { field: "name", .. }) => Ok(DEFAULT_TREE_NAME.to_string()),
            Err(error) => Err(error),
        },
    }
}

fn resolve_provider(
    provider: Option<String>,
    prompt: &mut dyn SeedPrompt,
) -> Result<Provider, SeedError> {
    match provider {
        Some(provider) => {
            Provider::parse(&provider).ok_or(SeedError::UnknownProvider { raw: provider })
        }
        None => prompt.prompt_provider(),
    }
}

fn resolve_model(
    model: Option<String>,
    provider: Provider,
    prompt: &mut dyn SeedPrompt,
) -> Result<String, SeedError> {
    let model = match model {
        Some(model) if !model.trim().is_empty() => model.trim().to_string(),
        _ => prompt.prompt_model(provider)?,
    };

    if !is_supported_model(provider, &model) {
        return Err(SeedError::UnsupportedModel { model, provider });
    }

    Ok(model)
}

fn canonical_seed_path(path: &Path) -> Result<PathBuf, SeedError> {
    let absolute = absolute_path(path)?;
    std::fs::create_dir_all(&absolute).map_err(|error| {
        SeedError::CreateTreeDir(format!("failed to create tree directory: {error}"))
    })?;
    std::fs::canonicalize(&absolute).map_err(|error| {
        SeedError::CreateTreeDir(format!("failed to resolve tree directory: {error}"))
    })
}

fn comparable_path(path: &Path) -> Result<PathBuf, SeedError> {
    let absolute = absolute_path(path)?;
    if absolute.exists() {
        return std::fs::canonicalize(&absolute).map_err(|error| {
            SeedError::CreateTreeDir(format!("failed to resolve tree directory: {error}"))
        });
    }
    Ok(normalize_path(&absolute))
}

fn absolute_path(path: &Path) -> Result<PathBuf, SeedError> {
    if path.as_os_str() == OsStr::new("") {
        return Err(SeedError::MissingInput {
            field: "path",
            flag: "--path <path>",
        });
    }

    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    std::env::current_dir()
        .map(|current| current.join(path))
        .map_err(|error| SeedError::CurrentDir(format!("failed to get current dir: {error}")))
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                normalized.push(component.as_os_str())
            }
        }
    }
    normalized
}

fn non_empty_or_default(value: String, default: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    }
}

fn supported_models(provider: Provider) -> Vec<&'static str> {
    models_for(provider).iter().map(|model| model.id).collect()
}

fn require_terminal(field: &'static str, flag: &'static str) -> Result<(), SeedError> {
    if io::stdin().is_terminal() {
        Ok(())
    } else {
        Err(SeedError::MissingInput { field, flag })
    }
}

fn read_prompt(message: &str) -> Result<String, SeedError> {
    eprint!("{message}");
    io::stderr()
        .flush()
        .map_err(|error| SeedError::PromptIo(format!("failed to write prompt: {error}")))?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .map_err(|error| SeedError::PromptIo(format!("failed to read prompt input: {error}")))?;
    Ok(input.trim().to_string())
}

#[cfg(test)]
#[path = "../tests/cli_seed_tests.rs"]
mod tests;
