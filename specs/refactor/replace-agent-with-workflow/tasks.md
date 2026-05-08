# Tasks: Replace Compile Agent Loop with Structured-Output Pipeline

## 1. Create `engine/llm/` module

- [x] Create `src/engine/llm/mod.rs` with `LlmProvider` trait, `LlmResponse` struct (content + finish_reason), `FinishReason` enum, `Message`, `Role` (System/User/Assistant only), `LlmError` enum
- [x] Create `src/engine/llm/providers/mod.rs` re-exporting `OpenAiProvider`
- [x] Create `src/engine/llm/providers/openai.rs` — chat completion with optional `response_format: json_schema`, maps API finish_reason to `FinishReason` enum, uses `max_completion_tokens` parameter, no tool-calling protocol
- [x] Add `pub mod llm` to `src/engine/mod.rs` (coexists with `pub mod agent` temporarily)
- [x] `cargo check` passes

## 2. Rewrite `cli/compile.rs`

- [x] Create `src/cli/compile.rs` as flat module replacing `src/cli/compile/mod.rs`
- [x] Implement guards (empty/single-leaf exit, missing API key) — same logic as current
- [x] Implement `read_valid_leaves()` — reads index + validates frontmatter, returns leaf content
- [x] Implement `compile_response_schema()` — returns the JSON Schema value for structured output
- [x] Implement `build_compile_prompt()` — system prompt + user message with all leaf content in XML-fenced format (`<document filename="..." title="...">body</document>`)
- [x] Implement `call_llm()` — constructs provider, sends single request with `max_completion_tokens: 16384`, checks `FinishReason::Length` before parse, returns response string
- [x] Implement `parse_and_validate()` — deserializes JSON, validates filenames/slugs (post-slugification uniqueness), deduplicates leaves per branch, filters unknown leaves with warnings
- [x] Implement `execute_plan()` — writes branches via `domain::branch::write()`, updates frontmatter via `domain::frontmatter::patch_fields()`, accumulates summary
- [x] Implement `print_summary()` — same output format as current
- [x] Port existing tests: empty collection, single leaf, missing API key
- [x] Add unit tests for `parse_and_validate()`: valid response, unknown leaves filtered, duplicate slugs rejected, duplicate leaves deduplicated, empty branches `[]` handled gracefully
- [x] Update `src/cli/mod.rs` to reference `compile` as flat module
- [x] `cargo test` passes — all compile tests green

## 3. Delete `engine/agent/` + cleanup

- [x] Remove `src/engine/agent/` directory entirely
- [x] Remove `pub mod agent` from `src/engine/mod.rs`
- [x] Remove dead imports/re-exports from `src/lib.rs` if any
- [x] Audit `Cargo.toml` for unused dependencies
- [x] `cargo test && cargo clippy -- -D warnings` clean
