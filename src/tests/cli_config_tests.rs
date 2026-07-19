use super::*;
use crate::domain::tree::TreeConfig;
use crate::domain::Timestamp;
use crate::engine::config::{self as engine_config, Config};
use crate::engine::llm::Provider;
use std::path::PathBuf;
use tempfile::TempDir;

// ── config write tests ───────────────────────────────────────────────────────

fn temp_config_path(dir: &TempDir) -> PathBuf {
    dir.path().join(".bo").join("config.json")
}

fn test_timestamp() -> Timestamp {
    Timestamp::parse("2026-05-12T00:00:00Z").unwrap()
}

fn seeded_config() -> Config {
    Config {
        provider: Provider::OpenAI,
        tree: Some(TreeConfig {
            path: PathBuf::from("/tmp/tree"),
            name: "tree".to_string(),
            created_at: test_timestamp(),
        }),
        model: "gpt-4.1-mini".to_string(),
        synthesis_model: None,
        base_url: None,
    }
}

fn seeded_config_with_models() -> Config {
    Config {
        provider: Provider::OpenAI,
        tree: Some(TreeConfig {
            path: PathBuf::from("/tmp/tree"),
            name: "tree".to_string(),
            created_at: test_timestamp(),
        }),
        model: "gpt-4o-mini".to_string(),
        synthesis_model: Some("gpt-4.1-mini".to_string()),
        base_url: None,
    }
}

#[test]
fn write_absent_config_creates_default_with_model() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let result = write_config(
        WriteConfigOptions {
            provider: None,
            model: Some("gpt-4.1-mini".to_string()),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap();

    assert_eq!(result.status, "ok");
    assert_eq!(result.model, "gpt-4.1-mini");
    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.model, "gpt-4.1-mini");
}

#[test]
fn write_creates_config() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let result = write_config(
        WriteConfigOptions {
            provider: None,
            model: Some("gpt-4.1-mini".to_string()),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap();

    assert_eq!(result.status, "ok");
    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.model, "gpt-4.1-mini");
    assert!(loaded.synthesis_model.is_none());
    assert!(loaded.tree.is_none());
}

#[test]
fn write_model_with_existing_synthesis_model_preserves_fallback() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);
    engine_config::write_config(
        &Config {
            provider: Provider::OpenAI,
            tree: None,
            model: "gpt-4o-mini".to_string(),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap();

    // Writing only model should keep model=..., synthesis_model unchanged
    let result = write_config(
        WriteConfigOptions {
            provider: None,
            model: Some("gpt-4o".to_string()),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap();

    assert_eq!(result.status, "ok");
    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.model, "gpt-4o");
    assert_eq!(loaded.synthesis_model.as_deref(), None);
}

#[test]
fn write_synthesis_model_persists_only_synthesis_model() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let result = write_config(
        WriteConfigOptions {
            provider: None,
            model: None,
            synthesis_model: Some("gpt-4.1-mini".to_string()),
            base_url: None,
        },
        &path,
    )
    .unwrap();

    assert_eq!(result.status, "ok");
    let loaded = engine_config::read_config(&path).unwrap();
    assert!(loaded.model.is_empty());
    assert_eq!(loaded.synthesis_model.as_deref(), Some("gpt-4.1-mini"));
    assert!(loaded.tree.is_none());
}

#[test]
fn write_trims_model_value() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    write_config(
        WriteConfigOptions {
            provider: None,
            model: Some(" gpt-4.1-mini ".to_string()),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap();

    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.model, "gpt-4.1-mini");
}

#[test]
fn write_preserves_tree_metadata() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);
    engine_config::write_config(&seeded_config(), &path).unwrap();

    write_config(
        WriteConfigOptions {
            provider: None,
            model: Some("gpt-4.1-mini".to_string()),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap();

    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.model, "gpt-4.1-mini");
    let tree = loaded.tree.unwrap();
    assert_eq!(tree.path, PathBuf::from("/tmp/tree"));
    assert_eq!(tree.name, "tree");
}

#[test]
fn write_model_preserves_synthesis_model_and_tree_metadata() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);
    engine_config::write_config(&seeded_config_with_models(), &path).unwrap();

    write_config(
        WriteConfigOptions {
            provider: None,
            model: Some("gpt-4.1".to_string()),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap();

    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.model, "gpt-4.1");
    assert_eq!(loaded.synthesis_model.as_deref(), Some("gpt-4.1-mini"));
    let tree = loaded.tree.unwrap();
    assert_eq!(tree.path, PathBuf::from("/tmp/tree"));
    assert_eq!(tree.name, "tree");
}

#[test]
fn write_unsupported_model_is_usage_error() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let err = write_config(
        WriteConfigOptions {
            provider: None,
            model: Some("unknown-model".to_string()),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap_err();

    assert_eq!(err.exit_code(), 2);
    assert!(matches!(err, ConfigWriteError::UnsupportedModel { .. }));
}

#[test]
fn write_unsupported_model_for_deepseek_is_usage_error() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let err = write_config(
        WriteConfigOptions {
            provider: Some(Provider::Deepseek),
            model: Some("gpt-4o".to_string()),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap_err();

    assert_eq!(err.exit_code(), 2);
    assert!(matches!(err, ConfigWriteError::UnsupportedModel { .. }));
}

#[test]
fn write_unsupported_synthesis_model_is_usage_error() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let err = write_config(
        WriteConfigOptions {
            provider: None,
            model: None,
            synthesis_model: Some("unknown-model".to_string()),
            base_url: None,
        },
        &path,
    )
    .unwrap_err();

    assert_eq!(err.exit_code(), 2);
    assert!(matches!(err, ConfigWriteError::UnsupportedModel { .. }));
}

#[test]
fn write_malformed_config_is_not_overwritten() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not json").unwrap();

    let err = write_config(
        WriteConfigOptions {
            provider: None,
            model: Some("gpt-4.1-mini".to_string()),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap_err();

    assert_eq!(err.exit_code(), 1);
    assert!(matches!(err, ConfigWriteError::Read(_)));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "not json");
}

#[test]
fn write_deepseek_provider() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let result = write_config(
        WriteConfigOptions {
            provider: Some(Provider::Deepseek),
            model: Some("deepseek-v4-flash".to_string()),
            synthesis_model: None,
            base_url: None,
        },
        &path,
    )
    .unwrap();

    assert_eq!(result.provider, "deepseek");
    assert_eq!(result.model, "deepseek-v4-flash");
}

#[test]
fn render_human_includes_provider() {
    let rendered = render_human(&ConfigWriteResult {
        status: "ok".to_string(),
        provider: "openai".to_string(),
        model: "gpt-4.1-mini".to_string(),
        synthesis_model: None,
        base_url: None,
    });

    assert!(rendered.contains("provider: openai"));
    assert!(rendered.contains("model: gpt-4.1-mini"));
}

#[test]
fn render_human_omits_none_synthesis_model() {
    let rendered = render_human(&ConfigWriteResult {
        status: "ok".to_string(),
        provider: "openai".to_string(),
        model: "gpt-4.1-mini".to_string(),
        synthesis_model: None,
        base_url: None,
    });

    assert!(!rendered.contains("synthesis_model"));
}

// ── custom provider / base_url ───────────────────────────────────────────────

#[test]
fn write_custom_provider_with_base_url_and_model() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let result = write_config(
        WriteConfigOptions {
            provider: Some(Provider::Custom),
            model: Some("my-fine-tune".to_string()),
            base_url: Some("https://api.example.com/v1".to_string()),
            ..Default::default()
        },
        &path,
    )
    .unwrap();

    assert_eq!(result.provider, "custom");
    assert_eq!(result.model, "my-fine-tune");
    assert_eq!(
        result.base_url.as_deref(),
        Some("https://api.example.com/v1")
    );
    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.provider, Provider::Custom);
    assert_eq!(
        loaded.base_url.as_deref(),
        Some("https://api.example.com/v1")
    );
}

#[test]
fn write_custom_provider_without_base_url_is_usage_error() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let err = write_config(
        WriteConfigOptions {
            provider: Some(Provider::Custom),
            model: Some("my-fine-tune".to_string()),
            ..Default::default()
        },
        &path,
    )
    .unwrap_err();

    assert_eq!(err.exit_code(), 2);
    assert!(matches!(err, ConfigWriteError::MissingBaseUrl));
    assert!(!path.exists());
}

#[test]
fn write_base_url_with_non_custom_provider_is_usage_error() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let err = write_config(
        WriteConfigOptions {
            provider: Some(Provider::OpenAI),
            model: Some("gpt-4.1-mini".to_string()),
            base_url: Some("https://api.example.com/v1".to_string()),
            ..Default::default()
        },
        &path,
    )
    .unwrap_err();

    assert_eq!(err.exit_code(), 2);
    assert!(matches!(
        err,
        ConfigWriteError::BaseUrlRequiresCustom { .. }
    ));
}

#[test]
fn write_invalid_base_url_is_usage_error() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let err = write_config(
        WriteConfigOptions {
            provider: Some(Provider::Custom),
            model: Some("my-fine-tune".to_string()),
            base_url: Some("not a url".to_string()),
            ..Default::default()
        },
        &path,
    )
    .unwrap_err();

    assert_eq!(err.exit_code(), 2);
    assert!(matches!(err, ConfigWriteError::InvalidBaseUrl { .. }));
}

#[test]
fn write_model_change_preserves_existing_base_url() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);
    write_config(
        WriteConfigOptions {
            provider: Some(Provider::Custom),
            model: Some("my-fine-tune".to_string()),
            base_url: Some("https://api.example.com/v1".to_string()),
            ..Default::default()
        },
        &path,
    )
    .unwrap();

    write_config(
        WriteConfigOptions {
            model: Some("another-model".to_string()),
            ..Default::default()
        },
        &path,
    )
    .unwrap();

    let loaded = engine_config::read_config(&path).unwrap();
    assert_eq!(loaded.model, "another-model");
    assert_eq!(
        loaded.base_url.as_deref(),
        Some("https://api.example.com/v1")
    );
}

#[test]
fn render_human_includes_base_url() {
    let rendered = render_human(&ConfigWriteResult {
        status: "ok".to_string(),
        provider: "custom".to_string(),
        model: "my-fine-tune".to_string(),
        synthesis_model: None,
        base_url: Some("https://api.example.com/v1".to_string()),
    });

    assert!(rendered.contains("base_url: https://api.example.com/v1"));
}
