# Implementation: Replace Compile Agent Loop with Structured-Output Pipeline

## Summary

Replaced the internal agent loop in `bo compile` with a deterministic pipeline that makes a single structured-output LLM call. The agent module (`engine/agent/`, ~1,424 LOC) was deleted entirely and replaced with a simplified LLM calling module (`engine/llm/`, ~212 LOC) and a flat compile pipeline (`cli/compile.rs`, ~874 LOC).

Net result: −338 LOC, single API call per compile, identical external behavior.

## What Changed

### Deleted

- `src/engine/agent/mod.rs` — agent loop, Tool trait, dispatch_tool(), Message with tool_calls
- `src/engine/agent/providers/openai.rs` — tool-calling protocol marshalling
- `src/engine/agent/tools/` — ListIndexTool, ReadLeafTool, WriteBranchTool, UpdateLeafFrontmatterTool
- `src/cli/compile/mod.rs` — directory-based module that orchestrated the agent

### Created

- `src/engine/llm/mod.rs` — `LlmProvider` trait, `LlmResponse`, `FinishReason`, `Message`, `Role`, `LlmError`
- `src/engine/llm/providers/openai.rs` — OpenAI chat completions with `response_format: json_schema` (structured output)
- `src/cli/compile.rs` — flat pipeline: read → prompt → call → validate → write

### Modified

- `src/engine/mod.rs` — `pub mod agent` → `pub mod llm`
- `milestones/oss-release.md` — added architectural stance section
- `scratchpad/features.md` — added migration task, updated compile-related items

## Architecture

```
cmd_compile()
  │
  ├─ read_valid_leaves()        — fs reads, frontmatter parse (deterministic)
  ├─ build_user_message()       — XML-fenced document serialization (deterministic)
  ├─ compile_response_schema()  — static JSON Schema
  ├─ call_llm()                 — single OpenAI API call, 16384 max_completion_tokens
  ├─ parse_and_validate()       — JSON deser + filename/slug validation (deterministic)
  ├─ execute_plan()             — branch::write() + frontmatter::patch_fields() (deterministic)
  └─ print_summary()
```

The LLM's only job is pattern identification and synthesis. Everything else is deterministic Rust code.

## Key Design Decisions

1. **`LlmProvider` returns `LlmResponse { content, finish_reason }`** — enables truncation detection before JSON parse attempt.
2. **`max_completion_tokens: 16384`** — up from 4096 to handle multi-branch responses.
3. **XML-fenced prompt format** — `<document filename="..." title="...">body</document>` avoids delimiter collision with markdown.
4. **`strict: true` on JSON schema** — API guarantees response conforms to schema, eliminating parse failures from malformed output.
5. **Post-slugification uniqueness check** — catches collisions like "Rust Ownership" vs "Rust: Ownership".
6. **`{"branches": []}` is valid** — prints "compiled: no branches found", still resets leaf frontmatter.

## Dogfood Result

Tested against 3 Rust Book chapters:

```
compiled: 3 branches across 3 leaves
  ✓ ownership-in-rust (2 leaves)
  ✓ references-and-borrowing-in-rust (2 leaves)
  ✓ traits-in-rust (1 leaf)
```

Single API call, ~5 seconds total. Branches contain synthesised markdown with cross-references. Leaf frontmatter correctly backlinked.

## Test Coverage

- 13 compile-specific unit tests (3 guard tests ported, 8 parse_and_validate, 2 execute_plan)
- 5 integration tests (ignored without API key, unchanged)
- All 158 unit tests passing, clippy clean
