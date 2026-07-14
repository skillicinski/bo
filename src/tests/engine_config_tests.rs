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
        model: "gpt-4.1-mini".to_string(),
        compile_model: None,
        base_url: None,
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
    assert_eq!(parsed["model"], "gpt-4.1-mini");
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
        model: "gpt-4.1-mini".to_string(),
        compile_model: None,
        base_url: None,
    };
    write_config(&config, &path).unwrap();

    let loaded = read_config(&path).unwrap();
    assert_eq!(loaded.model, "gpt-4.1-mini");
    assert_eq!(loaded.effective_model().unwrap().as_str(), "gpt-4.1-mini");
}

#[test]
fn compile_model_roundtrip_with_value() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let config = Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree("/tmp/bo")),
        model: "gpt-4o-mini".to_string(),
        compile_model: Some("gpt-4.1-mini".to_string()),
        base_url: None,
    };
    write_config(&config, &path).unwrap();

    let loaded = read_config(&path).unwrap();
    assert_eq!(loaded.model, "gpt-4o-mini");
    assert_eq!(loaded.compile_model.as_deref(), Some("gpt-4.1-mini"));
    assert_eq!(loaded.effective_model().unwrap().as_str(), "gpt-4o-mini");
    assert_eq!(
        loaded.effective_compile_model().unwrap().as_str(),
        "gpt-4.1-mini"
    );
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
        model: "gpt-4.1-mini".to_string(),
        compile_model: None,
        base_url: None,
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
    assert_eq!(loaded.model, "gpt-4.1-mini");
    assert_eq!(loaded.effective_model().unwrap().as_str(), "gpt-4.1-mini");
}

#[test]
fn write_config_without_tree_omits_tree_key() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    write_config(
        &Config {
            provider: Provider::OpenAI,
            tree: None,
            model: "gpt-4.1-mini".to_string(),
            compile_model: None,
            base_url: None,
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
        model: "gpt-4.1-mini".to_string(),
        compile_model: None,
        base_url: None,
    };

    let seeded = cfg.into_seeded().unwrap();

    assert_eq!(seeded.tree().path, PathBuf::from("/tmp/bo"));
    assert_eq!(
        seeded.config.effective_model().unwrap().as_str(),
        "gpt-4.1-mini"
    );
}

#[test]
fn seeded_conversion_fails_when_tree_missing() {
    let cfg = Config {
        provider: Provider::OpenAI,
        tree: None,
        model: "gpt-4.1-mini".to_string(),
        compile_model: None,
        base_url: None,
    };

    assert!(cfg.into_seeded().is_none());
}

#[test]
fn seeded_config_effective_model_fails_with_empty_model() {
    let cfg = Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree("/tmp/bo")),
        model: String::new(),
        compile_model: None,
        base_url: None,
    };

    let seeded = cfg.into_seeded().unwrap();

    assert!(seeded.config.effective_model().is_err());
    assert!(seeded.config.effective_compile_model().is_err());
}

#[test]
fn seeded_config_effective_compile_model_prefers_compile_model() {
    let cfg = Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree("/tmp/bo")),
        model: "gpt-4o-mini".to_string(),
        compile_model: Some("gpt-4.1-mini".to_string()),
        base_url: None,
    };

    let seeded = cfg.into_seeded().unwrap();

    assert_eq!(
        seeded.config.effective_model().unwrap().as_str(),
        "gpt-4o-mini"
    );
    assert_eq!(
        seeded.config.effective_compile_model().unwrap().as_str(),
        "gpt-4.1-mini"
    );
}

#[test]
fn seeded_config_effective_compile_model_falls_back_to_model() {
    let cfg = Config {
        provider: Provider::OpenAI,
        tree: Some(make_tree("/tmp/bo")),
        model: "gpt-4o-mini".to_string(),
        compile_model: None,
        base_url: None,
    };

    let seeded = cfg.into_seeded().unwrap();

    assert_eq!(
        seeded.config.effective_compile_model().unwrap().as_str(),
        "gpt-4o-mini"
    );
}

#[test]
fn base_url_roundtrip_and_omitted_when_none() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    write_config(&make_seeded_config("/tmp/bo"), &path).unwrap();
    let contents = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert!(parsed.get("base_url").is_none());

    let config = Config {
        provider: Provider::Custom,
        base_url: Some("https://api.example.com/v1".to_string()),
        ..make_seeded_config("/tmp/bo")
    };
    write_config(&config, &path).unwrap();

    let loaded = read_config(&path).unwrap();
    assert_eq!(
        loaded.base_url.as_deref(),
        Some("https://api.example.com/v1")
    );
}
