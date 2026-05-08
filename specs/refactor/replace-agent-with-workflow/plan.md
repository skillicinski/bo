# Plan: Replace Compile Agent Loop with Structured-Output Pipeline

## Architecture Decisions

### 1. `engine/agent/` → `engine/llm/`

The module provides LLM calling capability, not agency. Rename and simplify:

- **Drop:** `Tool` trait, `ToolCall` struct, `Role::Tool`, `tool_calls_raw` on Message, `run()` agent loop, `dispatch_tool()`
- **Keep:** `LlmProvider` trait (simplified), `Message`, `Role` (System/User/Assistant only), `Completion` (content only, no tool_calls)
- **Keep:** `providers/openai.rs` (rewritten for structured output instead of tool-calling)

### 2. OpenAI provider: tool-calling → structured output

Current provider marshals tool schemas and parses tool_call arrays. New provider:
- Sends a system + user message pair
- Sets `response_format: { type: "json_schema", schema: <compile_schema> }` 
- Receives a single JSON string in `completion.content`
- No tool protocol overhead

The `LlmProvider` trait becomes:

```rust
#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn complete(
        &self,
        messages: &[Message],
        model: &str,
        max_tokens: u32,
        response_schema: Option<&Value>,  // JSON Schema for structured output
    ) -> Result<LlmResponse, LlmError>;
}

pub struct LlmResponse {
    pub content: String,
    pub finish_reason: FinishReason,
}

pub enum FinishReason {
    Stop,
    Length,       // output truncated — max_completion_tokens hit
    ContentFilter,
    Other(String),
}
```

Returns content plus finish_reason so callers can detect truncation before attempting parse. The compile pipeline checks for `FinishReason::Length` and surfaces a specific error.

### 3. `cli/compile/mod.rs` (directory) → `cli/compile.rs` (flat file)

The pipeline:

```rust
pub fn cmd_compile(cfg: &Config) -> Result<(), String> {
    // Guards (unchanged): empty/single-leaf early exit, API key check
    let leaves = read_valid_leaves(cfg)?;           // deterministic
    let prompt = build_compile_prompt(&leaves);      // deterministic
    let schema = compile_response_schema();          // static JSON Schema
    let response = call_llm(cfg, &prompt, &schema)?; // one API call
    let plan = parse_and_validate(response, &leaves)?; // deterministic
    let summary = execute_plan(plan, cfg)?;           // write branches + update frontmatter
    print_summary(&summary);
    Ok(())
}
```

Private helpers handle:
- **Branch writing:** validate leaf filenames against known set, call `domain::branch::write()`, accumulate results — logic lifted from WriteBranchTool::execute
- **Frontmatter updating:** path-traversal guard, call `domain::frontmatter::patch_fields()`, write — logic lifted from UpdateLeafFrontmatterTool::execute
- **Validation:** reject unknown leaf filenames (filter with warning), deduplicate leaves per branch, check slug uniqueness post-slugification, ensure response schema conformance before any writes
- **Empty result handling:** `{"branches": []}` is valid — prints "compiled: no branches found", still updates all leaf frontmatter with `branches: []`

### Prompt Format

Leaf content is serialized into the user message using XML-style document fencing:

```
<document filename="understanding-ownership.md" title="Understanding Ownership">
{full markdown body}
</document>
```

This is unambiguous, the LLM can reference filenames directly, and it avoids delimiter collision with markdown content.

### Output Token Budget

`max_completion_tokens` is set to 16384 (up from 4096). A compile response with 10+ branches, each containing a multi-paragraph markdown body, can easily exceed 4096 tokens. The pipeline detects truncation via `FinishReason::Length` and surfaces a specific error rather than attempting to parse truncated JSON.

### 4. Delete `engine/agent/` entirely

All 1100 LOC removed. No migration path needed — the new module shares no types with the old one.

## Key Components

| Component | Location | Responsibility |
|-----------|----------|----------------|
| `LlmProvider` trait | `engine/llm/mod.rs` | Provider-agnostic async LLM calling |
| `OpenAiProvider` | `engine/llm/providers/openai.rs` | OpenAI chat completions with optional JSON schema response format |
| `LlmError` | `engine/llm/mod.rs` | Network/API/parse errors (replaces AgentError) |
| Compile pipeline | `cli/compile.rs` | Orchestrates the full compile: read → prompt → call → validate → write |
| Compile prompt | `cli/compile.rs` (const/fn) | System prompt instructing concept extraction |
| Response schema | `cli/compile.rs` (fn) | JSON Schema defining the structured output shape |
| Branch writing | `cli/compile.rs` (private fn) | Validates + delegates to `domain::branch::write()` |
| Frontmatter updating | `cli/compile.rs` (private fn) | Validates + delegates to `domain::frontmatter::patch_fields()` |

## Integration Points

- **OpenAI API:** structured output via `response_format` parameter (supported in gpt-4o, gpt-4o-mini). Falls back gracefully if model doesn't support it (API error surfaces cleanly).
- **`domain::branch`:** existing `write()` and `read_compiled_at()` used unchanged.
- **`domain::frontmatter`:** existing `patch_fields()` used unchanged.
- **`domain::index`:** existing `read_index()` used unchanged.
- **`domain::leaf`:** existing `read_frontmatter()` used for validation.
- **`engine::config`:** `effective_compile_model()` used unchanged.

## Implementation Sequence

1. Create `engine/llm/mod.rs` + `engine/llm/providers/mod.rs` + `engine/llm/providers/openai.rs`
2. Rewrite `cli/compile.rs` as flat module using new `engine::llm`
3. Update `engine/mod.rs`: replace `pub mod agent` with `pub mod llm`
4. Update `lib.rs` if needed (check re-exports)
5. Delete `src/engine/agent/` directory
6. Run `cargo test` — all existing compile tests should pass
7. Verify `cargo clippy` clean
8. Check Cargo.toml for dead dependencies

## Context Window Overflow Handling

If the assembled prompt (system + all leaf content) exceeds the model's context limit, the OpenAI API will return an error. The pipeline surfaces this as:

```
error: collection too large for model context window (N tokens estimated, model limit is M)
```

Token estimation can be approximate (chars/4 heuristic) for the initial implementation. Precise tokenization is out of scope.

## Test Strategy

- **Existing tests preserved:** empty collection, single leaf, missing API key — these test guards that don't touch the LLM.
- **New unit tests:** `parse_and_validate()` with good/bad JSON fixtures — tests the structured output parsing and validation logic in isolation.
- **Integration test shape:** mock LlmProvider returning canned JSON → verify branches written + frontmatter updated correctly. This was impossible with the agent loop (entangled with tool dispatch); now trivial.
