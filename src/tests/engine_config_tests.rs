use super::*;
use tempfile::TempDir;

fn temp_config_path(dir: &TempDir) -> PathBuf {
    dir.path().join(".bo").join("config.json")
}

fn make_tree(output_dir: &str) -> TreeConfig {
    TreeConfig {
        output_dir: PathBuf::from(output_dir),
        name: None,
        created_at: None,
    }
}

fn make_seeded_config(output_dir: &str) -> Config {
    Config {
        tree: Some(make_tree(output_dir)),
        model: None,
    }
}

#[test]
fn write_then_read_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    write_config(&make_seeded_config("/tmp/my-tree"), &path).unwrap();

    let loaded = read_config(&path).unwrap();
    assert_eq!(
        loaded.tree.unwrap().output_dir,
        PathBuf::from("/tmp/my-tree")
    );
}

#[test]
fn written_file_is_valid_json_with_tree_key() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    write_config(&make_seeded_config("/some/path"), &path).unwrap();

    let contents = std::fs::read_to_string(&path).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert_eq!(parsed["tree"]["output_dir"], "/some/path");
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
        tree: Some(make_tree("/tmp/bo")),
        model: Some("gpt-4.1-mini".to_string()),
    };
    write_config(&config, &path).unwrap();

    let loaded = read_config(&path).unwrap();
    assert_eq!(loaded.model.as_deref(), Some("gpt-4.1-mini"));
    assert_eq!(loaded.effective_model(), "gpt-4.1-mini");
}

#[test]
fn name_and_created_at_roundtrip() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    let config = Config {
        tree: Some(TreeConfig {
            output_dir: PathBuf::from("/tmp/bo"),
            name: Some("my-research".to_string()),
            created_at: Some("2026-04-14T09:00:00Z".to_string()),
        }),
        model: None,
    };
    write_config(&config, &path).unwrap();

    let loaded = read_config(&path).unwrap();
    let tree = loaded.tree.unwrap();
    assert_eq!(tree.name.as_deref(), Some("my-research"));
    assert_eq!(tree.created_at.as_deref(), Some("2026-04-14T09:00:00Z"));
}

#[test]
fn model_absent_uses_default() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, r#"{"tree":{"output_dir":"/tmp/bo"}}"#).unwrap();

    let loaded = read_config(&path).unwrap();
    assert!(loaded.model.is_none());
    assert_eq!(loaded.effective_model(), DEFAULT_MODEL);
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
    assert_eq!(loaded.effective_model(), "gpt-4.1-mini");
}

#[test]
fn write_config_without_tree_omits_tree_key() {
    let dir = TempDir::new().unwrap();
    let path = temp_config_path(&dir);

    write_config(
        &Config {
            tree: None,
            model: Some("gpt-4.1-mini".to_string()),
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
        tree: Some(make_tree("/tmp/bo")),
        model: Some("gpt-4.1-mini".to_string()),
    };

    let seeded = cfg.into_seeded().unwrap();

    assert_eq!(seeded.tree.output_dir, PathBuf::from("/tmp/bo"));
    assert_eq!(seeded.effective_model(), "gpt-4.1-mini");
}

#[test]
fn seeded_conversion_fails_when_tree_missing() {
    let cfg = Config {
        tree: None,
        model: Some("gpt-4.1-mini".to_string()),
    };

    assert!(cfg.into_seeded().is_none());
}

#[test]
fn seeded_config_uses_default_model_when_absent() {
    let cfg = Config {
        tree: Some(make_tree("/tmp/bo")),
        model: None,
    };

    let seeded = cfg.into_seeded().unwrap();

    assert_eq!(seeded.effective_model(), DEFAULT_MODEL);
}
