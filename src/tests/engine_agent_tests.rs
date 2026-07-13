//! Generic agent loop tests with a scripted tool-calling provider.
//!
//! These exercise the public agent entry point (`run_agent`) against a
//! deterministic provider that returns a scripted sequence of `AgentResponse`s.
//! Covers termination, unknown/malformed tools, validation feedback, output
//! truncation, context preflight, the hard turn limit, reasoning replay, usage
//! accumulation, and the no-mixed-terminal rule.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::engine::llm::{
    AgentMessage, AgentResponse, FinishReason, LlmError, LlmProvider, NormalizedSchema, ToolCall,
    ToolSchema, Usage,
};

use super::{run_agent, AgentOutcome, AgentRun, Tool, ToolError, ToolOutcome, MAX_TURNS};

// ── scripted provider ────────────────────────────────────────────────────────

struct ScriptedProvider {
    responses: Vec<AgentResponse>,
    calls: AtomicUsize,
    last_messages: Mutex<Vec<AgentMessage>>,
}

impl ScriptedProvider {
    fn new(responses: Vec<AgentResponse>) -> Self {
        Self {
            responses,
            calls: AtomicUsize::new(0),
            last_messages: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn last_messages(&self) -> Vec<AgentMessage> {
        self.last_messages.lock().expect("poisoned").clone()
    }
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    async fn complete(
        &self,
        _: &[crate::engine::llm::Message],
        _: &str,
        _: u32,
        _: Option<&NormalizedSchema>,
        _: bool,
    ) -> Result<crate::engine::llm::LlmResponse, LlmError> {
        unimplemented!("agent tests use complete_with_tools")
    }

    async fn complete_with_tools(
        &self,
        messages: &[AgentMessage],
        _: &str,
        _: u32,
        _: &[ToolSchema],
        _: bool,
    ) -> Result<AgentResponse, LlmError> {
        *self.last_messages.lock().expect("poisoned") = messages.to_vec();
        let i = self.calls.fetch_add(1, Ordering::SeqCst);
        self.responses
            .get(i)
            .cloned()
            .ok_or_else(|| LlmError::Api("scripted provider exhausted".to_string()))
    }
}

// ── response builders ────────────────────────────────────────────────────────

fn tool_call(id: &str, name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: args.to_string(),
    }
}

fn with_tools(calls: Vec<ToolCall>) -> AgentResponse {
    AgentResponse {
        content: None,
        reasoning_content: None,
        tool_calls: calls,
        finish_reason: FinishReason::Other("tool_calls".to_string()),
        usage: None,
    }
}

fn with_tools_and_usage(calls: Vec<ToolCall>, usage: Usage) -> AgentResponse {
    AgentResponse {
        content: None,
        reasoning_content: None,
        tool_calls: calls,
        finish_reason: FinishReason::Other("tool_calls".to_string()),
        usage: Some(usage),
    }
}

fn truncated_with_tools(calls: Vec<ToolCall>) -> AgentResponse {
    AgentResponse {
        content: None,
        reasoning_content: None,
        tool_calls: calls,
        finish_reason: FinishReason::Length,
        usage: None,
    }
}

fn reasoning_with_tools(calls: Vec<ToolCall>, reasoning: &str) -> AgentResponse {
    AgentResponse {
        content: None,
        reasoning_content: Some(reasoning.to_string()),
        tool_calls: calls,
        finish_reason: FinishReason::Other("tool_calls".to_string()),
        usage: None,
    }
}

// ── test tools ───────────────────────────────────────────────────────────────

/// A shared call counter so a test can inspect how often a tool ran even after
/// the tool is boxed and moved into the agent run.
type Counter = Arc<AtomicUsize>;

fn new_counter() -> Counter {
    Arc::new(AtomicUsize::new(0))
}

/// A tool that returns a fixed content result and counts its calls.
struct EchoTool {
    name: String,
    result: String,
    calls: Counter,
}

impl EchoTool {
    fn new(name: &str, result: &str, calls: Counter) -> Self {
        Self {
            name: name.to_string(),
            result: result.to_string(),
            calls,
        }
    }
}

impl Tool for EchoTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: self.name.clone(),
            description: "echo tool".to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }
    }
    fn execute(&self, arguments: &str) -> Result<ToolOutcome, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if arguments.trim().is_empty() {
            return Err(ToolError("empty arguments".to_string()));
        }
        let _ = serde_json::from_str::<serde_json::Value>(arguments)
            .map_err(|e| ToolError(format!("invalid arguments: {e}")))?;
        Ok(ToolOutcome::Content(self.result.clone()))
    }
}

/// A terminal tool. Fails (validation feedback) when arguments contain "fail",
/// otherwise terminates the loop. Mirrors submit_compile's contract.
struct TerminalTool {
    calls: Counter,
}

impl TerminalTool {
    fn new(calls: Counter) -> Self {
        Self { calls }
    }
}

impl Tool for TerminalTool {
    fn name(&self) -> &str {
        "submit"
    }
    fn schema(&self) -> ToolSchema {
        ToolSchema {
            name: "submit".to_string(),
            description: "terminal tool".to_string(),
            parameters: serde_json::json!({"type": "object", "properties": {}}),
        }
    }
    fn is_terminal(&self) -> bool {
        true
    }
    fn execute(&self, arguments: &str) -> Result<ToolOutcome, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if arguments.contains("fail") {
            return Err(ToolError("validation failed: bad plan".to_string()));
        }
        Ok(ToolOutcome::Terminate("plan accepted".to_string()))
    }
}

fn run(provider: &ScriptedProvider, tools: Vec<Box<dyn Tool>>) -> AgentOutcome {
    let run = AgentRun {
        provider,
        model: "test-model",
        system_prompt: "system".to_string(),
        user_message: "user".to_string(),
        tools,
        reasoning_disabled: false,
    };
    run_agent(run)
}

fn boxed<T: Tool + 'static>(tool: T) -> Box<dyn Tool> {
    Box::new(tool)
}

// ── tests ────────────────────────────────────────────────────────────────────

#[test]
fn valid_terminal_submission_completes() {
    let submit_calls = new_counter();
    let provider = ScriptedProvider::new(vec![with_tools(vec![tool_call("c1", "submit", "{}")])]);
    let outcome = run(
        &provider,
        vec![boxed(TerminalTool::new(submit_calls.clone()))],
    );

    assert!(
        matches!(
            outcome,
            AgentOutcome::Completed {
                turns: 1,
                tool_calls: 1,
                ..
            }
        ),
        "expected Completed, got {outcome:?}"
    );
    assert_eq!(provider.call_count(), 1);
    assert_eq!(submit_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn unknown_tool_returns_error_result_and_consumes_a_turn() {
    let provider = ScriptedProvider::new(vec![
        with_tools(vec![tool_call("c1", "does_not_exist", "{}")]),
        with_tools(vec![tool_call("c2", "submit", "{}")]),
    ]);
    let outcome = run(&provider, vec![boxed(TerminalTool::new(new_counter()))]);

    assert!(
        matches!(
            outcome,
            AgentOutcome::Completed {
                turns: 2,
                tool_calls: 2,
                ..
            }
        ),
        "expected Completed after feedback, got {outcome:?}"
    );
    // The unknown-tool error was fed back as a tool result keyed by call id.
    let messages = provider.last_messages();
    let tool_results: Vec<&AgentMessage> = messages
        .iter()
        .filter(|m| matches!(m, AgentMessage::Tool(_)))
        .collect();
    assert_eq!(tool_results.len(), 1);
    if let AgentMessage::Tool(result) = tool_results[0] {
        assert_eq!(result.tool_call_id, "c1");
        assert!(result.content.contains("unknown tool"));
    } else {
        panic!("expected a tool result");
    }
}

#[test]
fn malformed_tool_arguments_return_error_result() {
    let provider = ScriptedProvider::new(vec![
        with_tools(vec![tool_call("c1", "echo", "not-json")]),
        with_tools(vec![tool_call("c2", "submit", "{}")]),
    ]);
    let outcome = run(
        &provider,
        vec![
            boxed(EchoTool::new("echo", "ok", new_counter())),
            boxed(TerminalTool::new(new_counter())),
        ],
    );

    assert!(
        matches!(
            outcome,
            AgentOutcome::Completed {
                turns: 2,
                tool_calls: 2,
                ..
            }
        ),
        "expected Completed after malformed-args feedback, got {outcome:?}"
    );
    let messages = provider.last_messages();
    if let AgentMessage::Tool(result) = messages
        .iter()
        .find(|m| matches!(m, AgentMessage::Tool(r) if r.tool_call_id == "c1"))
        .expect("missing c1 tool result")
    {
        assert!(result.content.contains("invalid arguments"));
    }
}

#[test]
fn validation_failure_feeds_back_then_succeeds() {
    let provider = ScriptedProvider::new(vec![
        with_tools(vec![tool_call("c1", "submit", r#"{"plan":"fail"}"#)]),
        with_tools(vec![tool_call("c2", "submit", r#"{"plan":"good"}"#)]),
    ]);
    let outcome = run(&provider, vec![boxed(TerminalTool::new(new_counter()))]);

    assert!(
        matches!(
            outcome,
            AgentOutcome::Completed {
                turns: 2,
                tool_calls: 2,
                ..
            }
        ),
        "expected Completed after validation feedback, got {outcome:?}"
    );
}

#[test]
fn truncated_response_never_executes_tool_calls() {
    let echo_calls = new_counter();
    let provider = ScriptedProvider::new(vec![truncated_with_tools(vec![tool_call(
        "c1", "echo", "{}",
    )])]);
    let outcome = run(
        &provider,
        vec![boxed(EchoTool::new("echo", "ok", echo_calls.clone()))],
    );

    assert!(
        matches!(outcome, AgentOutcome::Truncated { turns: 1 }),
        "expected Truncated, got {outcome:?}"
    );
    assert_eq!(
        echo_calls.load(Ordering::SeqCst),
        0,
        "a truncated tool call must never execute"
    );
}

#[test]
fn plain_text_without_submission_ends_incomplete_at_turn_limit() {
    // Each turn: a non-terminal echo call. The loop hits MAX_TURNS and gives up.
    let responses: Vec<AgentResponse> = (0..MAX_TURNS)
        .map(|i| with_tools(vec![tool_call(&format!("c{i}"), "echo", "{}")]))
        .collect();
    let provider = ScriptedProvider::new(responses);
    let outcome = run(
        &provider,
        vec![boxed(EchoTool::new("echo", "ok", new_counter()))],
    );

    match outcome {
        AgentOutcome::Incomplete {
            turns, tool_calls, ..
        } => {
            assert_eq!(turns, MAX_TURNS, "should exhaust all turns");
            assert_eq!(tool_calls, MAX_TURNS);
        }
        other => panic!("expected Incomplete, got {other:?}"),
    }
}

#[test]
fn context_preflight_fails_cleanly_on_overflow() {
    // A system prompt alone larger than the context ceiling overflows on the
    // first preflight, before any provider call.
    let huge_prompt = "x".repeat(600_000);
    let provider = ScriptedProvider::new(vec![with_tools(vec![tool_call("c1", "submit", "{}")])]);
    let run = AgentRun {
        provider: &provider,
        model: "test-model",
        system_prompt: huge_prompt,
        user_message: "user".to_string(),
        tools: vec![boxed(TerminalTool::new(new_counter()))],
        reasoning_disabled: false,
    };
    let outcome = run_agent(run);

    assert!(
        matches!(outcome, AgentOutcome::ContextOverflow { .. }),
        "expected ContextOverflow, got {outcome:?}"
    );
    assert_eq!(provider.call_count(), 0, "no provider call on overflow");
}

#[test]
fn mixing_terminal_with_other_tools_is_rejected() {
    let provider = ScriptedProvider::new(vec![
        with_tools(vec![
            tool_call("c1", "echo", "{}"),
            tool_call("c2", "submit", "{}"),
        ]),
        with_tools(vec![tool_call("c3", "submit", "{}")]),
    ]);
    let outcome = run(
        &provider,
        vec![
            boxed(EchoTool::new("echo", "ok", new_counter())),
            boxed(TerminalTool::new(new_counter())),
        ],
    );

    assert!(
        matches!(outcome, AgentOutcome::Completed { turns: 2, .. }),
        "expected Completed after rejecting the mixed turn, got {outcome:?}"
    );
    // The rejected turn fed back an error tool result for every call id, so the
    // transcript the second provider call received carries both c1 and c2.
    let messages = provider.last_messages();
    let tool_results: Vec<&str> = messages
        .iter()
        .filter_map(|m| match m {
            AgentMessage::Tool(r) => Some(r.tool_call_id.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        tool_results.contains(&"c1"),
        "expected an error result for c1 from the rejected turn: {tool_results:?}"
    );
    assert!(
        tool_results.contains(&"c2"),
        "expected an error result for c2 from the rejected turn: {tool_results:?}"
    );
}

#[test]
fn reasoning_content_is_replayed_in_transcript() {
    // Thinking-mode response carries reasoning_content. The agent must append it
    // to the assistant message so the provider can replay it on the next turn
    // (DeepSeek returns HTTP 400 if omitted). Turn 1 is a non-terminal echo so a
    // second provider call happens, carrying the replayed assistant message.
    let provider = ScriptedProvider::new(vec![
        reasoning_with_tools(
            vec![tool_call("c1", "echo", "{}")],
            "thinking about the plan",
        ),
        with_tools(vec![tool_call("c2", "submit", "{}")]),
    ]);
    let outcome = run(
        &provider,
        vec![
            boxed(EchoTool::new("echo", "ok", new_counter())),
            boxed(TerminalTool::new(new_counter())),
        ],
    );

    assert!(matches!(outcome, AgentOutcome::Completed { .. }));
    // The second provider call's transcript holds the replayed assistant turn.
    let messages = provider.last_messages();
    let has_reasoning = messages.iter().any(|m| matches!(m, AgentMessage::Assistant { reasoning_content: Some(rc), .. } if rc == "thinking about the plan"));
    assert!(
        has_reasoning,
        "expected reasoning_content replayed on the assistant message"
    );
}

#[test]
fn usage_is_accumulated_across_turns() {
    let provider = ScriptedProvider::new(vec![
        with_tools_and_usage(
            vec![tool_call("c1", "echo", "{}")],
            Usage {
                prompt_tokens: 100,
                completion_tokens: 10,
                total_tokens: 110,
            },
        ),
        with_tools_and_usage(
            vec![tool_call("c2", "submit", "{}")],
            Usage {
                prompt_tokens: 200,
                completion_tokens: 20,
                total_tokens: 220,
            },
        ),
    ]);
    let outcome = run(
        &provider,
        vec![
            boxed(EchoTool::new("echo", "ok", new_counter())),
            boxed(TerminalTool::new(new_counter())),
        ],
    );

    if let AgentOutcome::Completed {
        usage: Some(usage),
        turns: 2,
        ..
    } = outcome
    {
        assert_eq!(usage.prompt_tokens, 300);
        assert_eq!(usage.completion_tokens, 30);
        assert_eq!(usage.total_tokens, 330);
    } else {
        panic!("expected Completed with accumulated usage, got {outcome:?}");
    }
}
