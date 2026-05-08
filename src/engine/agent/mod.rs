// Agent loop — provider-agnostic LLM tool-calling infrastructure.
//
// Public surface: Tool + LlmProvider traits, AgentConfig, supporting types.
// Reusable tools live in the `tools` submodule.

pub mod providers;
pub mod tools;

pub use providers::OpenAiProvider;

// async-openai is an implementation detail — none of its types appear outside
// this module.  Swapping the backend means replacing OpenAiProvider below.
//
// Trait design mirrors mini-agent (https://github.com/RajMandaliya/mini-agent)
// for forward compatibility; migration is a drop-in if that crate matures.

use async_trait::async_trait;
use serde_json::Value;
use std::fmt;

// ── public types ──────────────────────────────────────────────────────────────

pub struct AgentConfig {
    pub api_key: String,
    pub model: String,
}

#[derive(Debug)]
pub enum AgentError {
    Network(String),
    Api(String),
    Parse(String),
    MaxSteps(usize),
}

impl fmt::Display for AgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AgentError::Network(s) => write!(f, "network error: {}", s),
            AgentError::Api(s) => write!(f, "API error: {}", s),
            AgentError::Parse(s) => write!(f, "response parse error: {}", s),
            AgentError::MaxSteps(n) => write!(f, "agent hit step limit ({} steps)", n),
        }
    }
}

// ── Tool trait ────────────────────────────────────────────────────────────────

/// A tool the agent may call.  Each bo compile tool is a struct implementing
/// this trait.
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn parameters_schema(&self) -> Value;
    async fn execute(&self, args: Value) -> Result<String, AgentError>;
}

// ── LlmProvider trait ─────────────────────────────────────────────────────────

/// An LLM backend.  Implement this to add a new provider.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn complete(
        &self,
        messages: &[Message],
        tools: &[&dyn Tool],
        model: &str,
    ) -> Result<Completion, AgentError>;
}

// ── Message / Completion / ToolCall ───────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
    /// Set on Tool-role messages to correlate with the assistant's tool call.
    pub tool_call_id: Option<String>,
    /// Raw JSON of the tool_calls array — stored so it can be echoed back to
    /// the API in the subsequent assistant message.
    pub tool_calls_raw: Option<Value>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            tool_call_id: None,
            tool_calls_raw: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            tool_call_id: None,
            tool_calls_raw: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub args: Value,
}

#[derive(Debug)]
pub struct Completion {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    /// Raw JSON of the tool_calls array for echoing back in the assistant turn.
    pub tool_calls_raw: Option<Value>,
}

// ── run() ─────────────────────────────────────────────────────────────────────

/// Run the agent loop.
///
/// Manages the full message history (system → user → assistant → tool result
/// turns).  Tool calls are dispatched sequentially — one `execute().await` at
/// a time — because the tools share mutable state via `Arc<Mutex>` and the
/// current_thread runtime makes parallelism unnecessary.
///
/// Returns `Ok(())` on clean termination (provider returns a turn with no
/// tool calls).  Returns `Err(AgentError::MaxSteps(n))` if the step budget is
/// exhausted before the agent finishes.
pub async fn run(
    provider: &dyn LlmProvider,
    tools: &[Box<dyn Tool>],
    config: &AgentConfig,
    system_prompt: &str,
    initial_message: &str,
    max_steps: usize,
) -> Result<(), AgentError> {
    let tool_refs: Vec<&dyn Tool> = tools.iter().map(|t| t.as_ref()).collect();

    let mut messages: Vec<Message> = vec![
        Message::system(system_prompt),
        Message::user(initial_message),
    ];

    for step in 0..max_steps {
        let completion = provider
            .complete(&messages, &tool_refs, &config.model)
            .await?;

        // Push the assistant turn (with raw tool_calls for API compliance)
        messages.push(Message {
            role: Role::Assistant,
            content: completion.content.clone().unwrap_or_default(),
            tool_call_id: None,
            tool_calls_raw: completion.tool_calls_raw.clone(),
        });

        if completion.tool_calls.is_empty() {
            // No tool calls — agent is done
            return Ok(());
        }

        // Dispatch tool calls sequentially
        for tc in &completion.tool_calls {
            let result = dispatch_tool(tc, tools).await;
            messages.push(Message {
                role: Role::Tool,
                content: result,
                tool_call_id: Some(tc.id.clone()),
                tool_calls_raw: None,
            });
        }

        let _ = step; // suppress unused warning on last iteration
    }

    Err(AgentError::MaxSteps(max_steps))
}

async fn dispatch_tool(tc: &ToolCall, tools: &[Box<dyn Tool>]) -> String {
    match tools.iter().find(|t| t.name() == tc.name) {
        Some(tool) => tool
            .execute(tc.args.clone())
            .await
            .unwrap_or_else(|e| format!("tool error: {}", e)),
        None => format!(
            "error: unknown tool '{}' — available: {}",
            tc.name,
            tools
                .iter()
                .map(|t| t.name())
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}
