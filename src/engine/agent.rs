//! Provider-neutral agent turn loop.
//!
//! One file holds the loop, the typed tool contract, the transcript, and the
//! fixed resource envelope. Compile-specific tools and orchestration live in
//! `cli::compile::agent`; this module imports no `cli` or `adapters` types
//! (`tests/architecture.rs` enforces that).
//!
//! Semantics copied from the pi protocol reference: preserve `tool_calls` and
//! matching `tool_call_id` results, replay `reasoning_content` after
//! thinking-mode tool calls, validate tool arguments before execution, and
//! never execute a tool call truncated by the output limit. Streaming,
//! parallel tool execution, sessions, compaction, and resume are out of scope.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::json;

use crate::engine::llm::{
    blocking_runtime, complete_with_tools_with_policy, AgentMessage, AgentResponse, FinishReason,
    LlmCallPolicy, LlmError, LlmProvider, ToolCall, ToolSchema, Usage,
};

// ── fixed resource envelope (not configuration) ──────────────────────────────
//
// A turn limit alone is insufficient: one turn may contain multiple tools and
// each provider call may retry. These constants bound every dimension named in
// the v0.0.10 design. Worst case = MAX_TURNS * AGENT_CALL_POLICY.max_attempts
// provider requests = 8 * 3 = 24, and MAX_TURNS * timeout = 8 * 180s wall-clock.

/// Maximum agent turns before the loop gives up.
pub(crate) const MAX_TURNS: usize = 8;
/// Maximum tool calls accepted in a single assistant response.
pub(crate) const MAX_TOOL_CALLS_PER_RESPONSE: usize = 8;
/// Maximum total tool calls across the whole run.
pub(crate) const MAX_TOTAL_TOOL_CALLS: usize = 48;
/// Per-call output token budget.
const PER_CALL_MAX_TOKENS: u32 = 8192;
/// Transcript byte ceiling; the loop fails cleanly on overflow (no compaction).
const MAX_CONTEXT_BYTES: usize = 512 * 1024;
/// Per-tool-result byte cap. Larger results are truncated with a notice.
const MAX_TOOL_RESULT_BYTES: usize = 16 * 1024;

const AGENT_CALL_POLICY: LlmCallPolicy = LlmCallPolicy {
    timeout: Duration::from_secs(180),
    max_attempts: 3,
    initial_backoff: Duration::from_secs(2),
};

// ── tool contract ────────────────────────────────────────────────────────────

/// A tool the agent can call. Implementations live in the composition layer
/// (e.g. `cli::compile::agent`); this trait is provider- and command-neutral.
pub(crate) trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> ToolSchema;
    /// Whether a successful call from this tool terminates the loop. A terminal
    /// tool may not be mixed with other tools in one response — the loop
    /// rejects such responses and feeds an error back to the model.
    fn is_terminal(&self) -> bool {
        false
    }
    /// Execute the tool. `arguments` is the raw JSON string from the model;
    /// implementations deserialize it into typed structs here (the tool
    /// boundary) and validate locally.
    fn execute(&self, arguments: &str) -> Result<ToolOutcome, ToolError>;
}

/// What a tool produces when executed.
pub(crate) enum ToolOutcome {
    /// Feed this content back to the model as a tool result and continue.
    Content(String),
    /// Stop the loop. The string is returned to the model as the final tool
    /// result for transcript completeness, then the run terminates.
    Terminate(String),
}

/// A tool execution failure. Converted to an error tool result that consumes a
/// turn — the model gets a chance to correct unknown/malformed calls and
/// validation failures.
pub(crate) struct ToolError(pub String);

// ── run outcome ──────────────────────────────────────────────────────────────

#[derive(Debug)]
pub(crate) enum AgentOutcome {
    /// A terminal tool completed successfully.
    Completed {
        turns: usize,
        tool_calls: usize,
        usage: Option<Usage>,
    },
    /// The loop ended without a valid terminal submission (plain text, no call).
    Incomplete {
        reason: String,
        turns: usize,
        tool_calls: usize,
        usage: Option<Usage>,
    },
    /// A hard limit (turns or total tool calls) was hit.
    LimitExceeded {
        reason: String,
        turns: usize,
        tool_calls: usize,
    },
    /// The provider truncated the response (finish_reason: length). Tool calls
    /// from a truncated response are never executed.
    Truncated { turns: usize },
    /// The serialized transcript exceeded the context ceiling.
    ContextOverflow { turns: usize },
    /// The provider call failed after retries.
    ProviderError { message: String, turns: usize },
}

/// Inputs for a single agent run.
pub(crate) struct AgentRun<'a> {
    pub provider: &'a dyn LlmProvider,
    pub model: &'a str,
    pub system_prompt: String,
    pub user_message: String,
    pub tools: Vec<Box<dyn Tool>>,
    pub reasoning_disabled: bool,
}

/// Drive the agent loop to completion on the shared blocking runtime.
pub(crate) fn run_agent(run: AgentRun<'_>) -> AgentOutcome {
    blocking_runtime().block_on(run_agent_async(run))
}

async fn run_agent_async(run: AgentRun<'_>) -> AgentOutcome {
    let mut transcript: Vec<AgentMessage> = vec![
        AgentMessage::System(run.system_prompt),
        AgentMessage::User(run.user_message),
    ];
    let tool_schemas: Vec<ToolSchema> = run.tools.iter().map(|t| t.schema()).collect();
    let tool_map: HashMap<&str, &dyn Tool> =
        run.tools.iter().map(|t| (t.name(), t.as_ref())).collect();

    let mut turns = 0usize;
    let mut total_tool_calls = 0usize;
    let mut usage: Option<Usage> = None;

    loop {
        if turns >= MAX_TURNS {
            return AgentOutcome::Incomplete {
                reason: format!("reached the {MAX_TURNS}-turn limit without submitting"),
                turns,
                tool_calls: total_tool_calls,
                usage,
            };
        }

        let context_bytes = transcript_byte_estimate(&transcript);
        if context_bytes > MAX_CONTEXT_BYTES {
            return AgentOutcome::ContextOverflow { turns };
        }

        turns += 1;
        let response = match complete_with_tools_with_policy(
            run.provider,
            &transcript,
            run.model,
            PER_CALL_MAX_TOKENS,
            &tool_schemas,
            run.reasoning_disabled,
            AGENT_CALL_POLICY,
        )
        .await
        {
            Ok(response) => response,
            Err(LlmError::RetryExhausted { last_error, .. }) => {
                return AgentOutcome::ProviderError {
                    message: last_error.to_string(),
                    turns,
                }
            }
            Err(error) => {
                return AgentOutcome::ProviderError {
                    message: error.to_string(),
                    turns,
                }
            }
        };

        accumulate_usage(&mut usage, response.usage.as_ref());

        // Never execute tool calls from a length-truncated response.
        if matches!(response.finish_reason, FinishReason::Length) {
            return AgentOutcome::Truncated { turns };
        }

        if response.tool_calls.is_empty() {
            // Plain assistant text without a terminal submission. Append and
            // continue; this consumes the turn and ends as Incomplete at the
            // turn limit.
            transcript.push(AgentMessage::Assistant {
                content: response.content,
                reasoning_content: response.reasoning_content,
                tool_calls: Vec::new(),
            });
            continue;
        }

        // Reject a response that mixes a terminal tool with other tool calls.
        let has_terminal = response.tool_calls.iter().any(|tc| {
            tool_map
                .get(tc.name.as_str())
                .is_some_and(|t| t.is_terminal())
        });
        if has_terminal && response.tool_calls.len() > 1 {
            transcript.push(assistant_message(&response));
            for tc in &response.tool_calls {
                total_tool_calls += 1;
                push_tool_result(
                    &mut transcript,
                    tc,
                    json!({"error": "a terminal tool must be the only tool call in a turn"})
                        .to_string(),
                );
            }
            if let Some(stop) = total_tool_call_limit(total_tool_calls, turns) {
                return stop;
            }
            continue;
        }

        // Reject a response with too many tool calls.
        if response.tool_calls.len() > MAX_TOOL_CALLS_PER_RESPONSE {
            transcript.push(assistant_message(&response));
            for tc in &response.tool_calls {
                total_tool_calls += 1;
                push_tool_result(
                    &mut transcript,
                    tc,
                    json!({
                        "error": format!(
                            "too many tool calls in one response ({}); limit is {}",
                            response.tool_calls.len(),
                            MAX_TOOL_CALLS_PER_RESPONSE
                        )
                    })
                    .to_string(),
                );
            }
            if let Some(stop) = total_tool_call_limit(total_tool_calls, turns) {
                return stop;
            }
            continue;
        }

        // Append the assistant turn with tool_calls + reasoning_content so the
        // provider can replay it on the next request.
        transcript.push(assistant_message(&response));

        // Execute tools sequentially. A valid terminal submission stops immediately.
        let mut terminated = false;
        for tc in &response.tool_calls {
            total_tool_calls += 1;
            match execute_tool(&tool_map, tc) {
                Ok(ToolOutcome::Content(content)) => {
                    push_tool_result(&mut transcript, tc, bound_result_bytes(content));
                }
                Ok(ToolOutcome::Terminate(message)) => {
                    push_tool_result(&mut transcript, tc, message);
                    terminated = true;
                    break;
                }
                Err(ToolError(message)) => {
                    push_tool_result(&mut transcript, tc, json!({"error": message}).to_string());
                }
            }
            if let Some(stop) = total_tool_call_limit(total_tool_calls, turns) {
                return stop;
            }
        }

        if terminated {
            return AgentOutcome::Completed {
                turns,
                tool_calls: total_tool_calls,
                usage,
            };
        }
    }
}

fn execute_tool(
    tool_map: &HashMap<&str, &dyn Tool>,
    call: &ToolCall,
) -> Result<ToolOutcome, ToolError> {
    match tool_map.get(call.name.as_str()) {
        Some(tool) => tool.execute(&call.arguments),
        None => Err(ToolError(format!("unknown tool: {}", call.name))),
    }
}

fn assistant_message(response: &AgentResponse) -> AgentMessage {
    AgentMessage::Assistant {
        content: response.content.clone(),
        reasoning_content: response.reasoning_content.clone(),
        tool_calls: response.tool_calls.clone(),
    }
}

fn push_tool_result(transcript: &mut Vec<AgentMessage>, call: &ToolCall, content: String) {
    transcript.push(AgentMessage::Tool(crate::engine::llm::ToolResult {
        tool_call_id: call.id.clone(),
        content,
    }));
}

/// Returns a `LimitExceeded` outcome if the total tool-call budget is exhausted.
fn total_tool_call_limit(total_tool_calls: usize, turns: usize) -> Option<AgentOutcome> {
    if total_tool_calls >= MAX_TOTAL_TOOL_CALLS {
        Some(AgentOutcome::LimitExceeded {
            reason: format!("reached the {MAX_TOTAL_TOOL_CALLS} total tool-call limit"),
            turns,
            tool_calls: total_tool_calls,
        })
    } else {
        None
    }
}

/// Truncate a tool result to the byte cap, appending a notice when trimmed.
fn bound_result_bytes(content: String) -> String {
    if content.len() <= MAX_TOOL_RESULT_BYTES {
        return content;
    }
    // ponytail: hard char-boundary cut; byte-safe because we re-collect from a
    // string split at a char boundary via floor_char_boundary.
    let cut = content.floor_char_boundary(MAX_TOOL_RESULT_BYTES);
    let mut truncated = String::from(&content[..cut]);
    truncated.push_str("\n[...result truncated at ");
    truncated.push_str(&MAX_TOOL_RESULT_BYTES.to_string());
    truncated.push_str(" bytes]");
    truncated
}

fn accumulate_usage(total: &mut Option<Usage>, response_usage: Option<&Usage>) {
    let Some(u) = response_usage else {
        return;
    };
    let slot = total.get_or_insert(Usage::default());
    slot.prompt_tokens += u.prompt_tokens;
    slot.completion_tokens += u.completion_tokens;
    slot.total_tokens += u.total_tokens;
}

/// Rough byte estimate of the serialized transcript for the context preflight.
/// Sums the string content a provider would send; not exact, but a faithful
/// ceiling check that fails cleanly on overflow.
fn transcript_byte_estimate(transcript: &[AgentMessage]) -> usize {
    transcript
        .iter()
        .map(|msg| match msg {
            AgentMessage::System(c) | AgentMessage::User(c) => c.len(),
            AgentMessage::Tool(result) => result.content.len() + result.tool_call_id.len(),
            AgentMessage::Assistant {
                content,
                reasoning_content,
                tool_calls,
            } => {
                let base = content.as_deref().map(str::len).unwrap_or(0)
                    + reasoning_content.as_deref().map(str::len).unwrap_or(0);
                let calls = tool_calls
                    .iter()
                    .map(|tc| tc.id.len() + tc.name.len() + tc.arguments.len())
                    .sum::<usize>();
                base + calls
            }
        })
        .sum()
}

#[cfg(test)]
#[path = "../tests/engine_agent_tests.rs"]
mod tests;
