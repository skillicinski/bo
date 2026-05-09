# Tasks: Leaf Summary Implementation

## Deterministic fallback

- [x] Create `src/engine/summary.rs` with `generate_fallback(body: &str) -> String` — split on whitespace, take first `SUMMARY_TARGET_WORDS` (200) words, rejoin with spaces.
- [x] Register module in `src/engine/mod.rs` (`pub mod summary;`).
- [x] Unit tests: normal body (>200 words truncates), body shorter than 200 words (returns as-is), empty body (returns empty string), whitespace normalization.

## Leaf writer extension

- [x] Extend `leaf::write` signature to accept `summary: Option<&str>`.
- [x] When `summary` is `Some`, write `summary: |` followed by the summary text indented with two spaces per line in the YAML frontmatter block.
- [x] Update all existing callers of `leaf::write` to pass `None` (no behavior change).
- [x] Unit tests: `leaf::write` with `Some(summary)` produces valid YAML that round-trips through `frontmatter::parse`, multi-line summaries with special characters (colons, quotes) are preserved, `None` produces no `summary:` field.

## Wire fallback into collect

- [x] In `write_new_document`, call `summary::generate_fallback(body_markdown)` and pass result to `leaf::write`.
- [x] Integration test: `collect_html` without `OPENAI_API_KEY` → leaf frontmatter contains `summary:` field with expected content (first ~200 words of body).
- [x] Verify existing collect tests still pass (summary field is additive, not breaking).

## LLM-powered summary

- [x] Add `generate_llm(body: &str, title: Option<&str>, provider: &dyn LlmProvider, model: &str) -> Result<String, LlmError>` — async function that builds prompt, truncates body to `SUMMARY_INPUT_MAX_WORDS` (4000) words, calls provider with response schema `{ "summary": string }`, parses and returns the summary string.
- [x] Add constants: `SUMMARY_TARGET_WORDS = 200`, `SUMMARY_INPUT_MAX_WORDS = 4000`.
- [x] Add system prompt and user message template per plan.md prompt design.
- [x] Unit test: body truncation logic (>4000 words is truncated, ≤4000 words is passed through).

## Orchestrator and collect wiring

- [x] Add `generate(body: &str, title: Option<&str>, model: &str) -> String` — checks `OPENAI_API_KEY` env var, if present: spins up tokio runtime, calls `generate_llm`, on error logs warning to stderr and falls back to `generate_fallback`. If no key: calls `generate_fallback` directly.
- [x] Replace direct `generate_fallback` call in `write_new_document` with `generate` orchestrator.
- [x] Print `"summarizing..."` to stderr when LLM path is taken (UX feedback during collect).
- [x] Verify LLM failure (bad key, network error) falls back gracefully without blocking collect.

## Integration test and dogfood

- [x] Integration test: `collect_html` with no API key produces a leaf with `summary:` field containing first ~200 words of body.
- [x] Manual dogfood: `bo collect <url>` with `OPENAI_API_KEY` set → verify LLM-generated summary in frontmatter is coherent ~200-word prose.
- [x] Verify `bo search` finds content in summary field (already searched as part of whole-file grep behavior).
