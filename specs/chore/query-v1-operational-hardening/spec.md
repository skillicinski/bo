# Spec: Query V1 operational hardening

## Problem statement

`bo query` V1 is functionally complete, but dogfood exposed operational problems that make the command fragile or lower quality in normal use:

- upstream extraction can store UI chrome as leaf titles, which makes query citations and ranking worse;
- LLM calls can hang indefinitely and do not retry transient provider failures;
- query context assembly uses a hardcoded budget with no model awareness;
- citation cleanup pulls in `regex` for one simple wikilink operation;
- citation metadata can diverge from valid wikilinks present in the final answer.

This chore hardens V1 without changing the user-facing query architecture. V1 remains deterministic retrieval plus one structured-output synthesis call.

## Requirements

### 1. Improve upstream title extraction quality for future collections

Collection must prefer content-specific titles over UI chrome titles when extracted metadata is polluted.

- Scope is future `bo collect` results only. Do not add repair/backfill behavior for existing polluted leaves or `index.jsonl` entries in this chore.
- Prefer actual page/document title signals over arbitrary denylist heuristics. Investigate and use available `trafilatura` metadata/options and HTML metadata/headings where they expose a better content title.
- When the extracted/metadata title is missing or clearly chrome-like, and the extracted markdown body contains a meaningful leading `#` or `##` heading, use that heading as the title.
- Ignore deeper headings for title selection.
- Preserve good metadata titles; do not replace them merely because a body heading exists.
- If no confident content-specific title is found, keep the existing extracted title rather than guessing.
- Ensure leaf frontmatter, slug generation, `index.jsonl`, and later query citations use the improved title.
- Add deterministic fixture coverage for an mdBook/Rust Book-like page where the polluted title would be `Keyboard shortcuts` and the expected collected title is `Understanding Ownership`.

### 2. Add bounded LLM call policy

LLM calls must not hang indefinitely and provider failure modes must be explicit to human and machine clients.

- Introduce a shared LLM call helper/policy in `engine::llm` that supports timeout and retry.
- Apply it to query synthesis, compile synthesis, and leaf summary generation.
- Keep timeout/retry constants clearly visible and named; do not bury the operational logic inside provider internals.
- Use per-call policy constants:
  - query synthesis: 60 seconds per attempt;
  - leaf summary generation: 30 seconds per attempt;
  - compile synthesis: 180 seconds per attempt;
  - all three: 3 total attempts maximum (initial attempt plus 2 retries).
- Retry only transient failures: timeout, HTTP 429/rate limit, HTTP 5xx, and clear network/transport failures.
- Do not retry schema parse failures, validation failures, content filter results, missing API keys, local configuration errors, or non-429 HTTP 4xx responses.
- Timeout/retry exhaustion is a failure. Do not silently convert an attempted provider timeout into deterministic fallback output.
- Query/compile/summary callers must treat provider truncation/`Length` and `ContentFilter` finish reasons as errors; they must not parse or use partial/blocked responses.
- Surface timeout, retry exhaustion, truncation, and content-filter failures as actionable human errors and structured JSON `llm_error`-style errors.

### 3. Make query context budgeting model-aware

Query must not silently assemble prompts larger than the configured model can accept.

- Replace the single hardcoded `60_000` word source budget with a budget derived from the configured `query_model`.
- Keep a small internal table of known model context windows:
  - `gpt-4o`: 128k tokens;
  - `gpt-4o-mini`: 128k tokens;
  - `gpt-4.1`: 1M tokens;
  - `gpt-4.1-mini`: 1M tokens;
  - `gpt-4.1-nano`: 1M tokens.
- Reserve space for the system prompt, user wrapper/question, and max completion tokens before allocating source context.
- Convert tokens to words conservatively; never exceed the computed source budget.
- Unknown `query_model` values must fail before the provider call with an unambiguous error explaining that bo cannot determine the model context window. Do not silently fall back to an 8k budget.
- If the known model's computed budget is too small for the minimum viable query prompt/context, fail before the provider call with an actionable error.
- Context-budget failures should exit as provider/config-style query failures (exit code 2) and expose a distinct JSON error code such as `context_budget_exhausted` or `unknown_model_context`.

### 4. Replace regex-based invalid citation cleanup and make citation metadata authoritative

Citation validation should not require a direct `regex` dependency and the Sources list/JSON citations must reflect valid citations in the final answer.

- Replace `Regex::replace_all` for invalid `[[slug]]` wikilinks with a small deterministic parser.
- Preserve valid wikilinks exactly as `[[slug]]`.
- For invalid but well-formed wikilinks, remove the brackets but keep the inner text.
- Leave malformed, nested, empty, or unclosed bracket text unchanged unless it is a well-formed wikilink the parser can safely classify.
- Build final citation metadata from the union of:
  1. valid wikilinks appearing in answer prose; and
  2. valid slugs returned in the structured `cited_slugs` field.
- Deduplicate citations in first-appearance order from answer prose, followed by any remaining valid `cited_slugs` in their returned order.
- Remove `regex` from direct dependencies if no direct uses remain.

## Success criteria

1. A Rust Book/mdBook fixture that previously produced `Keyboard shortcuts` now stores `Understanding Ownership` in leaf frontmatter, slug/index metadata, and query-visible citation metadata for future collections.
2. A fake hanging provider causes query/compile/summary pipeline tests to return timeout errors within bounded test durations.
3. Transient fake provider failures are retried up to 3 total attempts; permanent failures are not retried.
4. Provider truncation/`Length` and `ContentFilter` finish reasons produce clear human and JSON errors instead of partial output.
5. Query context assembly for known models stays below the model-derived budget.
6. Unknown `query_model` values fail before provider calls with an unambiguous context-window error.
7. Existing query retrieval/ranking behavior remains unchanged by this chore.
8. Invalid wikilink cleanup behavior is deterministic, final citation metadata includes valid prose-only wikilinks, citations are deduped in the specified order, and no direct `regex` dependency remains.
9. `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test` pass.

## Out of scope

- Splitting `src/cli/query.rs` into `src/cli/query/` submodules. Defer until the command grows enough to justify it.
- Query retrieval I/O optimization, two-phase retrieval, large-tree thresholds, and candidate tuning.
- Agentic query/navigation V2.
- Gemini, Anthropic, OpenRouter, local/OpenAI-compatible endpoint support.
- Cross-query sessions or conversational mode.
- Streaming output.
- `bo config set` or new config UX.
- Repair/backfill for already-collected leaves with bad titles.
- Full dogfood event ledger/regression framework.
- Tree survey/rebuild/prune/snapshot features.
- PDF/RSS/audio/other collection adapters.

## Open questions

None.
