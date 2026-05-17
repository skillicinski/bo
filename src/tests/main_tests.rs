use super::*;
use bo::domain::tree::TreeConfig;
use std::cell::Cell;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

#[test]
fn raw_json_mode_detection_stops_at_arg_terminator() {
    assert!(raw_json_mode_requested(&[
        OsString::from("bo"),
        OsString::from("search"),
        OsString::from("--json"),
    ]));
    assert!(!raw_json_mode_requested(&[
        OsString::from("bo"),
        OsString::from("search"),
        OsString::from("--"),
        OsString::from("--json"),
    ]));
}

#[test]
fn compile_validation_json_error_includes_next_action() {
    let error = compile_json_error(&CompileError::Validation(
        "invalid compile response".to_string(),
    ));

    assert_eq!(error.code, "validation_error");
    assert_eq!(error.message, "invalid compile response");
    assert_eq!(error.details["phase"], "compile_validation");
    assert_eq!(error.details["files_changed"], false);
    assert_eq!(error.details["next_step"], compile::VALIDATION_NEXT_STEP);
}

#[test]
fn query_json_error_includes_low_relevance_details() {
    let error = query_json_error(&query::QueryError::LowRelevance {
        reason: query::LowRelevanceReason::GenericQuery,
        matched_sources: 8,
    });

    assert_eq!(error.code, "low_relevance");
    assert_eq!(error.details["reason"], "generic_query");
    assert_eq!(error.details["matched_sources"], 8);
    assert!(error.details["next_step"]
        .as_str()
        .unwrap()
        .contains("specific"));
}

#[test]
fn query_json_no_answer_errors_include_next_steps() {
    let errors = vec![
        query::QueryError::EmptyTree,
        query::QueryError::NoResults,
        query::QueryError::InsufficientSources {
            leaves_consulted: 2,
        },
    ];

    for error in errors {
        let json_error = query_json_error(&error);
        assert_eq!(json_error.code, error.code());
        assert!(
            json_error.details["next_step"].is_string(),
            "missing next_step for {error:?}"
        );
    }
}

#[test]
fn query_preflight_no_answer_takes_precedence_over_missing_provider() {
    let empty = TempDir::new().unwrap();
    write_index(empty.path(), &[]);
    assert_no_provider_resolver_not_called(&seeded_config(empty.path()), "what is rust", |err| {
        matches!(err, query::QueryError::EmptyTree)
    });

    let no_results = TempDir::new().unwrap();
    write_leaf(
        no_results.path(),
        "cooking.md",
        "Cooking Tips",
        "Boil water and add salt.",
    );
    write_index(
        no_results.path(),
        &[(
            "leaves/cooking.md",
            "Cooking Tips",
            "https://example.com/cooking",
        )],
    );
    assert_no_provider_resolver_not_called(&seeded_config(no_results.path()), "rust", |err| {
        matches!(err, query::QueryError::NoResults)
    });

    let weak = TempDir::new().unwrap();
    write_leaf(
        weak.path(),
        "trust.md",
        "Trust Building",
        "Trust grows slowly.",
    );
    write_index(
        weak.path(),
        &[(
            "leaves/trust.md",
            "Trust Building",
            "https://example.com/trust",
        )],
    );
    assert_no_provider_resolver_not_called(&seeded_config(weak.path()), "rust", |err| {
        matches!(
            err,
            query::QueryError::LowRelevance {
                reason: query::LowRelevanceReason::WeakMatches,
                ..
            }
        )
    });
}

#[test]
fn query_relevant_sources_require_provider() {
    let dir = TempDir::new().unwrap();
    write_leaf(
        dir.path(),
        "only-leaf.md",
        "Only Leaf",
        "Rust is a language focused on safety.",
    );
    write_index(
        dir.path(),
        &[(
            "leaves/only-leaf.md",
            "Only Leaf",
            "https://example.com/only",
        )],
    );
    let calls = Cell::new(0);

    let err = execute_query_with_provider_resolver(
        &seeded_config(dir.path()),
        "what is rust safety",
        || {
            calls.set(calls.get() + 1);
            Err(query::QueryError::NoProvider(
                "missing provider".to_string(),
            ))
        },
    )
    .unwrap_err();

    assert!(matches!(err, query::QueryError::NoProvider(_)));
    assert_eq!(calls.get(), 1);
}

fn assert_no_provider_resolver_not_called(
    cfg: &SeededConfig,
    question: &str,
    matches_expected_error: impl FnOnce(query::QueryError) -> bool,
) {
    let calls = Cell::new(0);
    let err = execute_query_with_provider_resolver(cfg, question, || {
        calls.set(calls.get() + 1);
        Err(query::QueryError::NoProvider(
            "missing provider".to_string(),
        ))
    })
    .unwrap_err();

    assert!(matches_expected_error(err));
    assert_eq!(calls.get(), 0);
}

fn seeded_config(tree: &Path) -> SeededConfig {
    SeededConfig {
        tree: TreeConfig {
            output_dir: tree.to_path_buf(),
            name: Some("test-tree".to_string()),
            created_at: Some("2026-05-17T00:00:00Z".to_string()),
        },
        model: Some("gpt-4o".to_string()),
    }
}

fn write_leaf(tree: &Path, filename: &str, title: &str, body: &str) {
    let leaves_dir = tree.join("leaves");
    fs::create_dir_all(&leaves_dir).unwrap();
    fs::write(
        leaves_dir.join(filename),
        format!(
            "---\ntitle: \"{}\"\nurl: \"https://example.com/{}\"\nsummary: \"{}\"\n---\n\n{}\n",
            title, filename, title, body
        ),
    )
    .unwrap();
}

fn write_index(tree: &Path, entries: &[(&str, &str, &str)]) {
    let bo_dir = tree.join(".bo");
    fs::create_dir_all(&bo_dir).unwrap();
    let content = entries
        .iter()
        .map(|(file, title, url)| {
            format!(
                r#"{{"file":"{}","title":"{}","url":"{}"}}"#,
                file, title, url
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let suffix = if content.is_empty() { "" } else { "\n" };
    fs::write(bo_dir.join("index.jsonl"), format!("{content}{suffix}")).unwrap();
}
