# Plan: Leaf Summary Implementation

## Architecture

### Summary generation module: `src/engine/summary.rs`

New module responsible for generating summaries. Two functions:

- `generate_summary_llm(body: &str, title: Option<&str>, provider: &dyn LlmProvider, model: &str) -> Result<String, LlmError>` — async, single structured-output call
- `generate_summary_fallback(body: &str) -> String` — deterministic, first ~200 words truncated at word boundary

### Integration point: `src/cli/collect.rs`

After content extraction and quality checks pass, before `write_new_document`:

1. Check if `OPENAI_API_KEY` is set
2. If yes: spin up tokio runtime, call LLM summary, use result
3. If no (or on LLM failure): use deterministic fallback
4. Pass summary to `write_new_document`

Summary generation failures are non-fatal — fall back to deterministic extraction and log a warning.

### Modified: `src/domain/leaf.rs`

Extend `leaf::write` to accept an optional `summary: Option<&str>` parameter. When present, write `summary:` field in YAML frontmatter.

## Key components

| Component | Responsibility |
|-----------|---------------|
| `engine::summary::generate_llm` | Async LLM call with summary prompt, returns ~200-word prose |
| `engine::summary::generate_fallback` | First ~200 words of body, truncated at word boundary |
| `engine::summary::generate` | Orchestrator: tries LLM, falls back to deterministic |
| `domain::leaf::write` | Extended to accept and persist optional summary field |
| `cli::collect::write_new_document` | Passes summary through to leaf writer |

## Implementation strategy

### Phase 1: Deterministic fallback (no external dependency)

1. Add `engine::summary::generate_fallback` — split body on whitespace, take first 200 words, join.
2. Extend `leaf::write` signature to accept `summary: Option<&str>`.
3. Wire fallback into `write_new_document` — always produces a summary, even without LLM.

### Phase 2: LLM-powered summary

1. Add `engine::summary::generate_llm` — single structured-output call, same provider/runtime pattern as compile.
2. Design prompt: system message establishes role (summarizer for retrieval), user message contains title + body, response schema expects `{ "summary": "..." }`.
3. Add `engine::summary::generate` orchestrator — checks API key, tries LLM, catches errors and falls back.
4. Wire into collect pipeline: replace direct fallback call with orchestrator.

### Phase 3: Integration and testing

1. Unit tests for fallback (word counting, boundary cases, empty body).
2. Unit tests for prompt construction.
3. Integration test: collect with no API key produces deterministic summary in frontmatter.
4. Manual dogfood: collect with API key produces LLM summary.

## Integration points

- **`engine::llm::LlmProvider`** — reused trait, same as compile
- **`engine::llm::OpenAiProvider`** — reused provider
- **`engine::config::Config`** — reuses `effective_compile_model()` for now (future: `summary_model`)
- **`OPENAI_API_KEY` env var** — same detection pattern as compile
- **`tokio::runtime::Runtime`** — same single-threaded block_on pattern as compile
- **No new crates**

## Prompt design

System message:
```
You are a document summarizer. Produce a single prose paragraph of approximately 200 words that captures what the document is about: its key topics, main argument or thesis, and what makes it distinctive. The summary should be optimized for retrieval — a reader should be able to determine whether to read the full document based on your summary alone. Do not include meta-commentary like "This document discusses..." — write directly about the content.
```

User message:
```
<title>{title}</title>
<document>
{body, truncated to ~4000 words to stay within cheap model limits}
</document>
```

Response schema:
```json
{
  "type": "object",
  "properties": {
    "summary": { "type": "string" }
  },
  "required": ["summary"],
  "additionalProperties": false
}
```

## YAML storage

Multi-line summaries use YAML literal block scalar (`|`) to avoid quoting issues:

```yaml
---
title: "Understanding Ownership"
url: https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html
collected_at: 2026-05-09T22:00:00Z
updated_at: 2026-05-09T22:00:00Z
summary: |
  Ownership is Rust's central memory management mechanism, replacing garbage
  collection with compile-time rules enforced by the borrow checker. The chapter
  introduces the ownership system through stack vs heap allocation...
---
```

## Risks and mitigations

| Risk | Mitigation |
|------|-----------|
| LLM call fails (network, rate limit, bad response) | Non-fatal: warn and fall back to deterministic summary |
| LLM returns truncated summary (finish_reason: length) | Accept partial summary rather than failing — still better than fallback |
| Body too large for cheap model context | Truncate body to ~4000 words before sending to LLM |
| Multi-line summary breaks YAML | Use literal block scalar (`|`) or ensure proper quoting in `leaf::write` |
| Collect latency increase | Acceptable: one cheap model call (~200-500ms). Print "summarizing..." to stderr for UX |
