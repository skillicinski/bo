use super::*;
use crate::engine::config;
use crate::engine::llm::Provider;
use std::path::PathBuf;
use tempfile::TempDir;

#[derive(Default)]
struct PromptAnswers {
    path: Option<PathBuf>,
    name: Option<String>,
    provider: Option<Provider>,
    model: Option<String>,
}

impl SeedPrompt for PromptAnswers {
    fn prompt_path(&mut self) -> Result<PathBuf, SeedError> {
        self.path.take().ok_or(SeedError::MissingInput {
            field: "path",
            flag: "--path <path>",
        })
    }

    fn prompt_name(&mut self, default: &str) -> Result<String, SeedError> {
        Ok(self.name.take().unwrap_or_else(|| default.to_string()))
    }

    fn prompt_provider(&mut self) -> Result<Provider, SeedError> {
        self.provider.take().ok_or(SeedError::MissingInput {
            field: "provider",
            flag: "--provider <provider>",
        })
    }

    fn prompt_model(&mut self, _provider: Provider) -> Result<String, SeedError> {
        self.model.take().ok_or(SeedError::MissingInput {
            field: "model",
            flag: "--model <model>",
        })
    }
}

fn options(path: PathBuf) -> SeedOptions {
    SeedOptions {
        path: Some(path),
        name: Some("tree".to_string()),
        provider: Some("openai".to_string()),
        model: Some("gpt-4.1-mini".to_string()),
    }
}

#[test]
fn fresh_seed_writes_config_without_manifest() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");
    let mut prompt = PromptAnswers::default();

    let result = seed(options(tree_dir.clone()), &config_path, &mut prompt).unwrap();

    assert_eq!(result.status, SeedStatus::Created);
    assert_eq!(result.path, std::fs::canonicalize(&tree_dir).unwrap());
    assert_eq!(result.name, "tree");
    assert_eq!(result.provider, Provider::OpenAI);
    assert_eq!(result.model, "gpt-4.1-mini");
    assert!(!tree_dir.join(".bo/manifest.json").exists());

    let cfg = config::read_config(&config_path).unwrap();
    let tree = cfg.tree.unwrap();
    assert_eq!(tree.path, result.path);
    assert_eq!(tree.name, "tree");
    assert_eq!(tree.created_at, result.created_at);
    assert_eq!(cfg.provider, Provider::OpenAI);
    assert_eq!(cfg.model, "gpt-4.1-mini");
}

#[test]
fn prompt_mode_supplies_missing_seed_fields() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = tmp.path().join("prompt-tree");
    let config_path = tmp.path().join("config.json");
    let mut prompt = PromptAnswers {
        path: Some(tree_dir.clone()),
        name: Some("prompted".to_string()),
        provider: Some(Provider::Deepseek),
        model: Some("deepseek-v4-flash".to_string()),
    };

    let result = seed(
        SeedOptions {
            path: None,
            name: None,
            provider: None,
            model: None,
        },
        &config_path,
        &mut prompt,
    )
    .unwrap();

    assert_eq!(result.status, SeedStatus::Created);
    assert_eq!(result.path, std::fs::canonicalize(&tree_dir).unwrap());
    assert_eq!(result.name, "prompted");
    assert_eq!(result.provider, Provider::Deepseek);
    assert_eq!(result.model, "deepseek-v4-flash");
}

#[test]
fn missing_name_uses_default_without_blocking_noninteractive_seed() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");
    let mut prompt = PromptAnswers::default();

    let result = seed(
        SeedOptions {
            path: Some(tree_dir),
            name: None,
            provider: Some("openai".to_string()),
            model: Some("gpt-4.1-mini".to_string()),
        },
        &config_path,
        &mut prompt,
    )
    .unwrap();

    assert_eq!(result.name, "bo");
}

#[test]
fn existing_same_path_returns_already_exists_and_keeps_config_read_only() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");
    let mut prompt = PromptAnswers::default();

    let created = seed(options(tree_dir.clone()), &config_path, &mut prompt).unwrap();
    let mut changed_options = options(tree_dir);
    changed_options.name = Some("ignored".to_string());
    changed_options.model = Some("gpt-4o".to_string());

    let existing = seed(changed_options, &config_path, &mut prompt).unwrap();

    assert_eq!(existing.status, SeedStatus::AlreadyExists);
    assert_eq!(existing.path, created.path);
    assert_eq!(existing.name, "tree");
    assert_eq!(existing.model, "gpt-4.1-mini");

    let cfg = config::read_config(&config_path).unwrap();
    assert_eq!(cfg.tree.unwrap().name, "tree");
    assert_eq!(cfg.model, "gpt-4.1-mini");
}

#[test]
fn existing_different_path_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let first_tree = tmp.path().join("first");
    let second_tree = tmp.path().join("second");
    let config_path = tmp.path().join("config.json");
    let mut prompt = PromptAnswers::default();

    seed(options(first_tree), &config_path, &mut prompt).unwrap();
    let err = seed(options(second_tree), &config_path, &mut prompt).unwrap_err();

    assert!(matches!(err, SeedError::TreeAlreadySeeded { .. }));
    assert_eq!(err.exit_code(), 2);
}

#[cfg(unix)]
#[test]
fn different_path_rejection_canonicalizes_missing_requested_path() {
    let tmp = TempDir::new().unwrap();
    let real_parent = tmp.path().join("real");
    let linked_parent = tmp.path().join("linked");
    std::fs::create_dir(&real_parent).unwrap();
    std::os::unix::fs::symlink(&real_parent, &linked_parent).unwrap();
    let config_path = tmp.path().join("config.json");
    let mut prompt = PromptAnswers::default();

    let first_tree = real_parent.join("first");
    seed(options(first_tree.clone()), &config_path, &mut prompt).unwrap();
    let err = seed(
        options(linked_parent.join("second")),
        &config_path,
        &mut prompt,
    )
    .unwrap_err();

    match err {
        SeedError::TreeAlreadySeeded {
            existing_path,
            requested_path,
        } => {
            assert_eq!(existing_path, std::fs::canonicalize(first_tree).unwrap());
            assert_eq!(
                requested_path,
                std::fs::canonicalize(real_parent).unwrap().join("second")
            );
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[test]
fn invalid_config_is_overwritten_by_fresh_seed() {
    let tmp = TempDir::new().unwrap();
    let tree_dir = tmp.path().join("tree");
    let config_path = tmp.path().join("config.json");
    std::fs::write(&config_path, "not json").unwrap();
    let mut prompt = PromptAnswers::default();

    let result = seed(options(tree_dir), &config_path, &mut prompt).unwrap();

    assert_eq!(result.status, SeedStatus::Created);
    assert_eq!(
        config::read_config(&config_path)
            .unwrap()
            .tree
            .unwrap()
            .name,
        "tree"
    );
}

#[test]
fn unsupported_provider_and_model_are_usage_errors() {
    let tmp = TempDir::new().unwrap();
    let config_path = tmp.path().join("config.json");
    let mut prompt = PromptAnswers::default();

    let unknown_provider = seed(
        SeedOptions {
            path: Some(tmp.path().join("tree-a")),
            name: Some("tree".to_string()),
            provider: Some("unknown".to_string()),
            model: Some("gpt-4.1-mini".to_string()),
        },
        &config_path,
        &mut prompt,
    )
    .unwrap_err();
    assert!(matches!(
        unknown_provider,
        SeedError::UnknownProvider { .. }
    ));
    assert_eq!(unknown_provider.exit_code(), 2);

    let unsupported_model = seed(
        SeedOptions {
            path: Some(tmp.path().join("tree-b")),
            name: Some("tree".to_string()),
            provider: Some("deepseek".to_string()),
            model: Some("gpt-4.1-mini".to_string()),
        },
        &config_path,
        &mut prompt,
    )
    .unwrap_err();
    assert!(matches!(
        unsupported_model,
        SeedError::UnsupportedModel { .. }
    ));
    assert_eq!(unsupported_model.exit_code(), 2);
}

#[test]
fn render_human_is_readable_for_new_and_existing_seed() {
    let result = SeedResult {
        status: SeedStatus::AlreadyExists,
        path: PathBuf::from("/tmp/tree"),
        name: "tree".to_string(),
        created_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
        provider: Provider::OpenAI,
        model: "gpt-4.1-mini".to_string(),
        compile_model: None,
    };

    let rendered = render_human(&result);

    assert!(rendered.ends_with('\n'));
    assert!(rendered.contains("bo is already seeded"));
    assert!(rendered.contains("path: /tmp/tree"));
    assert!(rendered.contains("use `bo config` to change provider or model"));
}
