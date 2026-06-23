use super::*;
use crate::domain::Timestamp;
use crate::engine::llm::Provider;
use tempfile::TempDir;

fn temp_config_path(dir: &TempDir) -> PathBuf {
    dir.path().join(".bo").join("config.json")
}

fn test_timestamp() -> Timestamp {
    Timestamp::parse("2026-04-14T09:00:00Z").unwrap()
}

fn make_tree(path: &str) -> TreeConfig {
    TreeConfig {
        path: PathBuf::from(path),
        name: "bo".to_string(),
        created_at: test_timestamp(),
    }
}

fn make_seeded_config(path: &str) -> Config {
    Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree(path)),
        model: None,
        compile_model: None,
    }
}

#[test]
fn write_then_read_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    write_config(&make_seeded_config("/tmp/my-tree"), &path).unwrap();

    let loaded = read_config(&path).unwrap();
    assert_eq!(loaded.tree.unwrap().path, PathBuf::from("/tmp/my-tree"));
}

#[test]
fn written_file_is_valid_json_with_tree_key() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    write_config(&make_seeded_config("/some/path"), &path).unwrap();

    let contents = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert_eq!(parsed["tree"]["path"], "/some/path");
    assert!(parsed.get("model").is_none());
}

#[test]
fn read_nonexistent_returns_not_found() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let err = read_config(&path).unwrap_err();
    assert!(matches!(err, ConfigError::NotFound));
}

#[test]
fn read_malformed_json_returns_parse_error() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "not json at all").unwrap();

    let err = read_config(&path).unwrap_err();
    assert!(matches!(err, ConfigError::Parse(_)));
}

#[test]
fn model_roundtrip_with_value() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let config = Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree("/tmp/bo")),
        model: Some("gpt-4.1-mini".to_string()),
        compile_model: None,
    };
    write_config(&config, &path).unwrap();

    let loaded = read_config(&path).unwrap();
    assert_eq!(loaded.model.as_deref(), Some("gpt-4.1-mini"));
    assert_eq!(loaded.effective_model().unwrap(), "gpt-4.1-mini");
}

#[test]
fn compile_model_roundtrip_with_value() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let config = Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree("/tmp/bo")),
        model: Some("gpt-4o-mini".to_string()),
        compile_model: Some("gpt-4.1-mini".to_string()),
    };
    write_config(&config, &path).unwrap();

    let loaded = read_config(&path).unwrap();
    assert_eq!(loaded.model.as_deref(), Some("gpt-4o-mini"));
    assert_eq!(loaded.compile_model.as_deref(), Some("gpt-4.1-mini"));
    assert_eq!(loaded.effective_model().unwrap(), "gpt-4o-mini");
    assert_eq!(loaded.effective_compile_model().unwrap(), "gpt-4.1-mini");
}

#[test]
fn tree_metadata_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let config = Config {
        provider: Provider::OpenAI,
        tree: Some(TreeConfig {
            path: PathBuf::from("/tmp/bo"),
            name: "my-research".to_string(),
            created_at: test_timestamp(),
        }),
        model: None,
        compile_model: None,
    };
    write_config(&config, &path).unwrap();

    let loaded = read_config(&path).unwrap();
    let tree = loaded.tree.unwrap();
    assert_eq!(tree.name, "my-research");
    assert_eq!(tree.created_at, test_timestamp());
}

#[test]
fn config_without_tree_deserializes() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, r#"{"model":"gpt-4.1-mini"}"#).unwrap();

    let loaded = read_config(&path).unwrap();
    assert!(loaded.tree.is_none());
    assert_eq!(loaded.model.as_deref(), Some("gpt-4.1-mini"));
    assert_eq!(loaded.effective_model().unwrap(), "gpt-4.1-mini");
}

#[test]
fn write_config_without_tree_omits_tree_key() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    write_config(
        &Config {
            provider: Provider::OpenAI,
            tree: None,
            model: Some("gpt-4.1-mini".to_string()),
            compile_model: None,
        },
        &path,
    )
    .unwrap();

    let contents = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert!(parsed.get("tree").is_none());
    assert_eq!(parsed["model"], "gpt-4.1-mini");
}

#[test]
fn seeded_conversion_succeeds_when_tree_exists() {
    let cfg = Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree("/tmp/bo")),
        model: Some("gpt-4.1-mini".to_string()),
        compile_model: None,
    };

    let seeded = cfg.into_seeded().unwrap();

    assert_eq!(seeded.tree().path, PathBuf::from("/tmp/bo"));
    assert_eq!(seeded.effective_model().unwrap(), "gpt-4.1-mini");
}

#[test]
fn seeded_conversion_fails_when_tree_missing() {
    let cfg = Config {
        provider: Provider::OpenAI,
        tree: None,
        model: Some("gpt-4.1-mini".to_string()),
        compile_model: None,
    };

    assert!(cfg.into_seeded().is_none());
}

#[test]
fn seeded_config_uses_default_model_when_absent() {
    let cfg = Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree("/tmp/bo")),
        model: None,
        compile_model: None,
    };

    let seeded = cfg.into_seeded().unwrap();

    assert_eq!(seeded.effective_model().unwrap(), "gpt-4.1-mini");
    assert_eq!(seeded.effective_compile_model().unwrap(), "gpt-4.1-mini");
}

#[test]
fn seeded_config_effective_compile_model_prefers_compile_model() {
    let cfg = Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree("/tmp/bo")),
        model: Some("gpt-4o-mini".to_string()),
        compile_model: Some("gpt-4.1-mini".to_string()),
    };

    let seeded = cfg.into_seeded().unwrap();

    assert_eq!(seeded.effective_model().unwrap(), "gpt-4o-mini");
    assert_eq!(seeded.effective_compile_model().unwrap(), "gpt-4.1-mini");
}

#[test]
fn seeded_config_effective_compile_model_falls_back_to_model() {
    let cfg = Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree("/tmp/bo")),
        model: Some("gpt-4o-mini".to_string()),
        compile_model: None,
    };

    let seeded = cfg.into_seeded().unwrap();

    assert_eq!(seeded.effective_compile_model().unwrap(), "gpt-4o-mini");
}
