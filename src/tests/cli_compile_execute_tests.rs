use super::*;
use crate::engine::llm::{Model, Provider};

#[test]
fn ensure_compile_context_fits_errors_on_overflow() {
    let model = Model::parse("gpt-4.1-mini", Provider::OpenAI).unwrap();

    let small = execute_prompt_tokens(64);
    assert!(ensure_compile_context_fits(&model, small).is_ok());

    let huge = execute_prompt_tokens(usize::MAX);
    assert!(
        ensure_compile_context_fits(&model, huge).is_err(),
        "overflow must error"
    );
}

/// Wrap a byte count into a token estimate comparable to what the compile
/// pipeline computes, so tests exercise the same fit-check path.
fn execute_prompt_tokens(prompt_bytes: usize) -> usize {
    estimate_compile_prompt_tokens(prompt_bytes)
}

#[test]
fn resource_limit_constants_have_expected_values() {
    assert_eq!(crate::engine::agent::MAX_TURNS, 8);
    assert_eq!(crate::engine::agent::MAX_TOOL_CALLS_PER_RESPONSE, 8);
    assert_eq!(crate::engine::agent::MAX_TOTAL_TOOL_CALLS, 48);
}
