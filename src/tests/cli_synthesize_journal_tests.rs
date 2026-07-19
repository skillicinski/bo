use super::*;
use crate::cli::synthesize::types::{SynthesisError, SynthesisMode};
use std::time::Duration;

#[test]
fn error_payload_routes_terminal_errors() {
    let slugs: &[String] = &[];
    let duration = Duration::from_millis(10);

    // Validation keeps its own shape: validation_failures, no error field.
    let payload = error_payload(
        SynthesisMode::Full,
        slugs,
        &SynthesisError::Validation("branch #1 has empty title".to_string()),
        duration,
    )
    .expect("validation is journaled");
    assert_eq!(
        payload.validation_failures,
        vec!["branch #1 has empty title".to_string()]
    );
    assert!(payload.error.is_none());

    // LLM/provider failures: error field, empty deltas.
    let llm_errors = [
        SynthesisError::Truncated,
        SynthesisError::ContentFilter,
        SynthesisError::Llm("upstream timeout".to_string()),
        SynthesisError::ContextOverflow {
            model: "gpt-4.1".to_string(),
            estimated_tokens: Some(200_000),
            context_tokens: Some(128_000),
        },
    ];
    for error in &llm_errors {
        let payload = error_payload(SynthesisMode::Full, slugs, error, duration)
            .expect("LLM/provider error is journaled");
        assert!(payload.validation_failures.is_empty());
        let err = payload.error.expect("error field present");
        assert!(!err.code.is_empty());
        assert!(!err.message.is_empty());
    }

    // Infrastructure / dry-run / agent failures are not synthesis verdicts.
    for error in [
        SynthesisError::Io("disk full".to_string()),
        SynthesisError::Busy("locked".to_string()),
        SynthesisError::DryRunBlocked("stale".to_string()),
        SynthesisError::AgentFailed {
            message: "limit".to_string(),
            turns: 0,
            tool_calls: 0,
            usage: None,
            signals_sent: 0,
            last_error: None,
        },
    ] {
        assert!(
            error_payload(SynthesisMode::Full, slugs, &error, duration).is_none(),
            "{:?} should not be journaled",
            error
        );
    }
}
