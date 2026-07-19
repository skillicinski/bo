//! Generic agent loop tests with a scripted tool-calling provider.
//!
//! These exercise the public agent entry point (`run_agent`) against a
//! deterministic provider that returns a scripted sequence of `AgentResponse`s.
//! Covers termination, unknown/malformed tools, validation feedback, output
//! truncation, context preflight, the hard turn limit, the total-tool-call
//! limit, reasoning replay, usage accumulation, last-error surfacing, and the
//! no-mixed-terminal rule.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use crate::engine::llm::{
    AgentMessage, AgentResponse, FinishReason, LlmError, LlmProvider, ProviderSchema, ToolCall,
    ToolSchema, Usage,
};

use super::{
    run_agent, AgentDiagnostics, AgentOutcome, AgentRun, Tool, ToolError, ToolOutcome,
    MAX_TOOL_CALLS_PER_RESPONSE, MAX_TOTAL_TOOL_CALLS, MAX_TURNS,
};

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
        _: Option<&ProviderSchema>,
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

/// Transcript-ordering conformance: every Assistant message with tool_calls
/// must be immediately followed by Tool messages matching each tool_call_id
/// before any message of another role (User, System, or another Assistant).
fn assert_transcript_ordering(messages: &[AgentMessage]) {
    let mut i = 0;
    while i < messages.len() {
        let tool_call_ids: Vec<&str> = match &messages[i] {
            AgentMessage::Assistant { tool_calls, .. } if !tool_calls.is_empty() => {
                tool_calls.iter().map(|tc| tc.id.as_str()).collect()
            }
            _ => {
                i += 1;
                continue;
            }
        };
        let mut found = std::collections::HashSet::new();
        i += 1;
        while i < messages.len() {
            match &messages[i] {
                AgentMessage::Tool(result) => {
                    if tool_call_ids.contains(&result.tool_call_id.as_str()) {
                        found.insert(result.tool_call_id.as_str());
                    } else {
                        panic!(
                            "unexpected tool result id {} after Assistant with tool_calls {:?}",
                            result.tool_call_id, tool_call_ids
                        );
                    }
                    i += 1;
                }
                _ => break, // non-Tool: done with this assistant's block
            }
        }
        let expected: std::collections::HashSet<&str> = tool_call_ids.iter().copied().collect();
        assert_eq!(
            found, expected,
            "Assistant tool_calls {:?} not followed by matching Tool messages before next non-Tool; got {:?}",
            tool_call_ids, found
        );
        // i already advanced past the Tool messages by the inner loop
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
    let outcome = run_agent(run);
    assert_transcript_ordering(&provider.last_messages());
    outcome
}

fn boxed<T: Tool + 'static>(tool: T) -> Box<dyn Tool> {
    Box::new(tool)
}

fn assert_completed(outcome: &AgentOutcome, turns: usize, tool_calls: usize) {
    assert!(
        matches!(outcome, AgentOutcome::Completed { .. }),
        "expected Completed, got {outcome:?}"
    );
    let diag: &AgentDiagnostics = outcome.diag();
    assert_eq!(diag.turns, turns, "turns: got {}", diag.turns);
    assert_eq!(
        diag.tool_calls, tool_calls,
        "tool_calls: got {}",
        diag.tool_calls
    );
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

    assert_completed(&outcome, 1, 1);
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

    assert_completed(&outcome, 2, 2);
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

    assert_completed(&outcome, 2, 2);
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

    assert_completed(&outcome, 2, 2);
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
        matches!(outcome, AgentOutcome::Truncated { .. }),
        "expected Truncated, got {outcome:?}"
    );
    assert_eq!(outcome.diag().turns, 1);
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
        AgentOutcome::Incomplete { diag, .. } => {
            assert_eq!(diag.turns, MAX_TURNS, "should exhaust all turns");
            assert_eq!(diag.tool_calls, MAX_TURNS);
        }
        other => panic!("expected Incomplete, got {other:?}"),
    }
}

#[test]
fn limit_exceeded_via_total_tool_calls() {
    // 8 tool calls per response × 6 turns = 48 = MAX_TOTAL_TOOL_CALLS. This is
    // the path two live runs hit on a 66-leaf tree: the model keeps calling
    // inspection tools without submitting. The loop must stop at the
    // total-tool-call limit, not run unbounded.
    let echo = EchoTool::new("echo", "ok", new_counter());
    let responses: Vec<AgentResponse> = (0..6)
        .map(|t| {
            let calls: Vec<ToolCall> = (0..8)
                .map(|j| tool_call(&format!("c{t}_{j}"), "echo", "{}"))
                .collect();
            with_tools(calls)
        })
        .collect();
    let provider = ScriptedProvider::new(responses);
    let outcome = run(&provider, vec![boxed(echo)]);

    match outcome {
        AgentOutcome::LimitExceeded { diag, .. } => {
            assert_eq!(
                diag.turns, 6,
                "should hit the total-tool-call limit on turn 6"
            );
            assert_eq!(diag.tool_calls, MAX_TOTAL_TOOL_CALLS);
        }
        other => panic!("expected LimitExceeded, got {other:?}"),
    }
}

#[test]
fn error_outcome_carries_last_tool_result_error() {
    // Turn 1: terminal tool fails validation (sets last_error). Then plain-text
    // turns run the loop to the turn limit. The Incomplete outcome must surface
    // the last validation message in its diagnostics.
    let provider = ScriptedProvider::new(vec![
        with_tools(vec![tool_call("c1", "submit", r#"{"plan":"fail"}"#)]),
        // Remaining turns: plain text (no tool calls) until MAX_TURNS.
        plain_text("thinking"),
        plain_text("thinking"),
        plain_text("thinking"),
        plain_text("thinking"),
        plain_text("thinking"),
        plain_text("thinking"),
        plain_text("thinking"),
    ]);
    let outcome = run(&provider, vec![boxed(TerminalTool::new(new_counter()))]);

    match outcome {
        AgentOutcome::Incomplete { diag, .. } => {
            assert_eq!(diag.turns, MAX_TURNS);
            assert_eq!(diag.tool_calls, 1);
            let last_error = diag
                .last_error
                .as_ref()
                .expect("expected the last validation message to be surfaced");
            assert!(
                last_error.contains("validation failed"),
                "expected validation message, got: {last_error}"
            );
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

    assert_transcript_ordering(&provider.last_messages());
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

    assert_completed(&outcome, 2, 3);
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

    match outcome {
        AgentOutcome::Completed { diag } => {
            assert_eq!(diag.turns, 2);
            let usage = diag.usage.expect("expected accumulated usage");
            assert_eq!(usage.prompt_tokens, 300);
            assert_eq!(usage.completion_tokens, 30);
            assert_eq!(usage.total_tokens, 330);
        }
        other => panic!("expected Completed with accumulated usage, got {other:?}"),
    }
}

#[test]
fn per_turn_cap_rejection_message() {
    // A response with > MAX_TOOL_CALLS_PER_RESPONSE tool calls feeds back an
    // error message containing "split your tool calls across multiple turns".
    let too_many = MAX_TOOL_CALLS_PER_RESPONSE + 1;
    let calls: Vec<ToolCall> = (0..too_many)
        .map(|i| tool_call(&format!("c{i}"), "echo", "{}"))
        .collect();
    let provider = ScriptedProvider::new(vec![
        with_tools(calls),
        with_tools(vec![tool_call("clast", "submit", "{}")]),
    ]);
    let outcome = run(
        &provider,
        vec![
            boxed(EchoTool::new("echo", "ok", new_counter())),
            boxed(TerminalTool::new(new_counter())),
        ],
    );

    // Second turn's submit should complete the run.
    assert_completed(&outcome, 2, too_many + 1);

    // The transcript sent on the second provider call includes the error
    // results from the rejected first turn.
    let messages = provider.last_messages();
    let has_cap_message = messages
        .iter()
        .filter_map(|m| match m {
            AgentMessage::Tool(r) => Some(r.content.as_str()),
            _ => None,
        })
        .any(|content| content.contains("split your tool calls across multiple turns"));
    assert!(
        has_cap_message,
        "expected tool result to contain the per-turn cap message"
    );
}

fn plain_text(text: &str) -> AgentResponse {
    AgentResponse {
        content: Some(text.to_string()),
        reasoning_content: None,
        tool_calls: Vec::new(),
        finish_reason: FinishReason::Stop,
        usage: None,
    }
}

// ── budget signal tests ─────────────────────────────────────────────────────

#[test]
fn soft_signal_at_75_percent_of_tool_calls() {
    // 5 turns of 8 echo calls (40 total). Soft fires at 36 (75%).
    // Turn 6 submits, completing the run with signals_sent=1.
    let mut responses: Vec<AgentResponse> = (0..5)
        .map(|t| {
            let calls: Vec<ToolCall> = (0..8)
                .map(|j| tool_call(&format!("c{t}_{j}"), "echo", "{}"))
                .collect();
            with_tools(calls)
        })
        .collect();
    responses.push(with_tools(vec![tool_call("c_final", "submit", "{}")]));
    let provider = ScriptedProvider::new(responses);
    let outcome = run(&provider, vec![boxed(TerminalTool::new(new_counter()))]);

    assert!(
        matches!(outcome, AgentOutcome::Completed { .. }),
        "expected Completed, got {outcome:?}"
    );
    assert_eq!(outcome.diag().signals_sent, 1, "expected soft signal only");

    // The submit turn's provider call carries the soft signal in its transcript.
    let messages = provider.last_messages();
    let has_soft = messages.iter().any(|m| match m {
        AgentMessage::User(content) => content.contains("Stop gathering"),
        _ => false,
    });
    assert!(has_soft, "expected soft signal in transcript");
}

#[test]
fn final_signal_at_90_percent_of_tool_calls() {
    // 5 turns of 7 echo calls = 35. Turn 6: 7 calls = 42 (soft at 36).
    // Turn 7: 7 calls, final fires at 44, limit at 48. signals_sent=2.
    let responses: Vec<AgentResponse> = (0..8)
        .map(|t| {
            let calls: Vec<ToolCall> = (0..7)
                .map(|j| tool_call(&format!("c{t}_{j}"), "echo", "{}"))
                .collect();
            with_tools(calls)
        })
        .collect();
    let provider = ScriptedProvider::new(responses);
    let outcome = run(
        &provider,
        vec![boxed(EchoTool::new("echo", "ok", new_counter()))],
    );

    assert!(
        matches!(outcome, AgentOutcome::LimitExceeded { .. }),
        "expected LimitExceeded, got {outcome:?}"
    );
    assert_eq!(
        outcome.diag().signals_sent,
        2,
        "expected both soft and final signals"
    );
    // The last provider call transcript carries at least the soft signal.
    let messages = provider.last_messages();
    let has_soft = messages.iter().any(|m| match m {
        AgentMessage::User(content) => content.contains("Stop gathering"),
        _ => false,
    });
    assert!(has_soft, "expected soft signal in last provider transcript");
}

#[test]
fn each_signal_at_most_once() {
    // After soft fires at total=36, subsequent calls in later turns should
    // not fire it again. 5 turns with 8 echo calls + 1 submit = soft only.
    let mut responses: Vec<AgentResponse> = (0..5)
        .map(|t| {
            let calls: Vec<ToolCall> = (0..8)
                .map(|j| tool_call(&format!("c{t}_{j}"), "echo", "{}"))
                .collect();
            with_tools(calls)
        })
        .collect();
    // After soft fires at 36 in turn 5, the final turn submits.
    responses.push(with_tools(vec![tool_call("c_final", "submit", "{}")]));
    let provider = ScriptedProvider::new(responses);
    let outcome = run(&provider, vec![boxed(TerminalTool::new(new_counter()))]);

    assert!(
        matches!(outcome, AgentOutcome::Completed { .. }),
        "expected Completed, got {outcome:?}"
    );
    // Exactly one soft signal, no duplicates.
    assert_eq!(outcome.diag().signals_sent, 1);
}

#[test]
fn signal_fires_on_plain_text_turns() {
    // Pure plain-text turns (no tool calls) consume turns and should still
    // cross the turn-based thresholds. 6 plain-text turns cross 75% of 8.
    let responses: Vec<AgentResponse> = (0..6).map(|_| plain_text("thinking")).collect();
    let provider = ScriptedProvider::new(responses);
    let outcome = run(&provider, vec![boxed(TerminalTool::new(new_counter()))]);

    // Provider exhausted at turn 7; soft fires at turn 6 (6/8 = 0.75).
    assert_eq!(
        outcome.diag().signals_sent,
        1,
        "expected soft signal on plain-text turns"
    );
}

#[test]
fn submits_after_signal_completes() {
    // Turn 1-4: 8 echo calls each (32 total). Turn 5: 8 echo calls (40 total),
    // soft signal fires at call 36. Turn 6: submit completes.
    let responses: Vec<AgentResponse> = vec![
        with_tools(
            (0..8)
                .map(|j| tool_call(&format!("c0_{j}"), "echo", "{}"))
                .collect(),
        ),
        with_tools(
            (0..8)
                .map(|j| tool_call(&format!("c1_{j}"), "echo", "{}"))
                .collect(),
        ),
        with_tools(
            (0..8)
                .map(|j| tool_call(&format!("c2_{j}"), "echo", "{}"))
                .collect(),
        ),
        with_tools(
            (0..8)
                .map(|j| tool_call(&format!("c3_{j}"), "echo", "{}"))
                .collect(),
        ),
        with_tools(
            (0..8)
                .map(|j| tool_call(&format!("c4_{j}"), "echo", "{}"))
                .collect(),
        ),
        with_tools(vec![tool_call("c_final", "submit", "{}")]),
    ];
    let provider = ScriptedProvider::new(responses);
    let outcome = run(&provider, vec![boxed(TerminalTool::new(new_counter()))]);

    assert!(
        matches!(outcome, AgentOutcome::Completed { .. }),
        "expected Completed, got {outcome:?}"
    );
    assert_eq!(outcome.diag().signals_sent, 1, "expected soft signal");
}

#[test]
fn no_signal_in_short_runs() {
    // A short run below thresholds: 1 echo call, then submit.
    let provider = ScriptedProvider::new(vec![
        with_tools(vec![tool_call("c1", "echo", "{}")]),
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
        matches!(outcome, AgentOutcome::Completed { .. }),
        "expected Completed, got {outcome:?}"
    );
    assert_eq!(outcome.diag().turns, 2);
    assert_eq!(outcome.diag().tool_calls, 2);
    assert_eq!(outcome.diag().signals_sent, 0, "no signals in short runs");
}

#[test]
fn signal_names_terminal_tool() {
    // Run up to the soft threshold and submit on the next turn, so the
    // signal appears in the submit turn's transcript.
    let mut responses: Vec<AgentResponse> = (0..5)
        .map(|t| {
            let calls: Vec<ToolCall> = (0..8)
                .map(|j| tool_call(&format!("c{t}_{j}"), "echo", "{}"))
                .collect();
            with_tools(calls)
        })
        .collect();
    responses.push(with_tools(vec![tool_call("c_final", "submit", "{}")]));
    let provider = ScriptedProvider::new(responses);
    let outcome = run(
        &provider,
        vec![
            boxed(EchoTool::new("echo", "ok", new_counter())),
            boxed(TerminalTool::new(new_counter())),
        ],
    );

    assert!(
        matches!(outcome, AgentOutcome::Completed { .. }),
        "expected Completed, got {outcome:?}"
    );
    assert_eq!(outcome.diag().signals_sent, 1);

    let messages = provider.last_messages();
    let has_named_tool = messages.iter().any(|m| match m {
        AgentMessage::User(content) => content.contains("`submit`"),
        _ => false,
    });
    assert!(
        has_named_tool,
        "expected signal to name the terminal tool `submit`"
    );
}
