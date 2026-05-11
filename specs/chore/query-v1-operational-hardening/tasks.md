# Tasks: Query V1 operational hardening

## 0. Preflight and stale-scope cleanup

- [x] Confirm `src/cli/query.rs` remains a single file for this chore; do not create `src/cli/query/` submodules
- [x] Confirm query retrieval semantics stay unchanged; do not add two-phase retrieval or large-tree thresholds
- [x] Run `rg "regex|Regex" Cargo.toml src` and note direct uses before citation cleanup
- [x] Run focused baseline tests: `cargo test cli_query engine_extract engine_summary`

## 1. Citation parser and direct regex removal

- [x] Add query citation tests for valid wikilinks preserved exactly
- [x] Add query citation tests for invalid well-formed wikilinks becoming plain inner text
- [x] Add query citation tests for adjacent wikilinks
- [x] Add query citation tests for malformed, nested, empty, and unclosed bracket text remaining conservative/unchanged
- [x] Add query citation test where a valid prose wikilink is missing from `cited_slugs` but appears in final citations
- [x] Add query citation test for dedupe/order: prose wikilinks first, then remaining valid `cited_slugs`
- [x] Replace `Regex::replace_all` citation cleanup in `src/cli/query.rs` with deterministic parser helpers
- [x] Build final citation metadata from valid answer wikilinks union valid structured `cited_slugs`
- [x] Remove direct `regex` dependency from `Cargo.toml`
- [x] Verify `Cargo.toml` has no direct `regex` entry and any `cargo tree -i regex` result is transitive only

## 2. Model-aware query context budget

- [x] Add known model context lookup for `gpt-4o`, `gpt-4o-mini`, `gpt-4.1`, `gpt-4.1-mini`, and `gpt-4.1-nano`
- [x] Add explicit query budget constants for max completion tokens, prompt overhead reserve, and minimum source words
- [x] Add query budget helper that reserves prompt/completion tokens and converts remaining source tokens to words conservatively
- [x] Add `QueryError::UnknownModelContext` for unknown `query_model` values
- [x] Add `QueryError::ContextBudgetExhausted` for known models with insufficient computed source budget
- [x] Update `assemble_context` to accept a computed source word budget instead of using fixed `TOKEN_BUDGET_WORDS`
- [x] Compute query context budget before retrieval/provider call in `run_with_provider`
- [x] Update `main.rs` query JSON error mapping for `unknown_model_context` and `context_budget_exhausted`
- [x] Add tests for known 128k and 1M model budgets
- [x] Add test that unknown model fails before provider invocation
- [x] Add test that exhausted/too-small budget fails before provider invocation
- [x] Update existing context assembly truncation tests to use explicit computed/test budgets

## 3. Shared LLM timeout/retry policy core

- [x] Extend `LlmError` with typed timeout and retry-exhaustion variants
- [x] Add or refine typed transient/permanent provider error variants needed for retry classification
- [x] Add `LlmCallPolicy { timeout, max_attempts, initial_backoff }` in `src/engine/llm`
- [x] Add `complete_with_policy(...)` using `tokio::time::timeout`
- [x] Add transient-error classification helper covering timeout, rate limit, 5xx, and network/transport failures
- [x] Ensure permanent errors are not retried: parse/schema, validation, content filter, truncation, missing API key/config, auth/quota/non-429 4xx
- [x] Add fake-provider test: succeeds after transient failure and is attempted exactly twice
- [x] Add fake-provider test: exhausts after 3 transient attempts
- [x] Add fake-provider test: hanging provider returns timeout within bounded test duration
- [x] Add fake-provider test: permanent error is attempted exactly once
- [x] Audit `async-openai` provider construction for hidden retry/backoff behavior
- [x] Configure or replace OpenAI provider call path so bo's visible 3-attempt policy is authoritative
- [x] Map OpenAI transport/rate-limit/server/permanent failures to typed `LlmError` categories as specifically as the client allows

## 4. Apply LLM policy to query synthesis

- [x] Add visible query LLM policy constant: 60s timeout per attempt, 3 total attempts
- [x] Update query synthesis to call `complete_with_policy`
- [x] Treat `FinishReason::Length` as a query error before parsing model content
- [x] Treat `FinishReason::ContentFilter` as a query error before parsing model content
- [x] Ensure query timeout/retry exhaustion renders actionable human error text
- [x] Ensure query timeout/retry exhaustion renders structured JSON `llm_error`-style output
- [x] Add query test for transient retry success with fake provider
- [x] Add query test for timeout failure with fake provider and short injected policy/helper
- [x] Add query test for `Length` finish reason failing without parsing partial content
- [x] Add query test for `ContentFilter` finish reason failing without parsing blocked content

## 5. Apply LLM policy to compile synthesis

- [x] Add visible compile LLM policy constant: 180s timeout per attempt, 3 total attempts
- [x] Extract a provider-injectable compile completion helper if needed for focused tests
- [x] Update compile LLM call to use `complete_with_policy`
- [x] Preserve existing `CompileError::ContextOverflow` behavior for maximum-context provider errors
- [x] Preserve `CompileError::Truncated` for `FinishReason::Length`
- [x] Preserve `CompileError::ContentFilter` for `FinishReason::ContentFilter`
- [x] Add compile test for transient retry success with fake provider
- [x] Add compile test for timeout failure with fake provider and short injected policy/helper
- [x] Add compile test for permanent failure not being retried
- [x] Add compile tests for `Length` and `ContentFilter` finish reasons producing existing compile errors

## 6. Apply LLM policy to summary and collect propagation

- [x] Add `SummaryError` for attempted-provider failures
- [x] Add visible summary LLM policy constant: 30s timeout per attempt, 3 total attempts
- [x] Update `summary::generate_llm` to call `complete_with_policy`
- [x] Treat `FinishReason::Length` as summary failure before parsing content
- [x] Treat `FinishReason::ContentFilter` as summary failure before parsing content
- [x] Change summary orchestrator so missing `OPENAI_API_KEY` still returns deterministic fallback successfully
- [x] Change summary orchestrator so attempted provider timeout/retry exhaustion/parse failure returns `Err`
- [x] Add `CollectError::Summary` or equivalent and display actionable error text
- [x] Update `collect::write_new_document` to abort before leaf/index writes when attempted summary generation fails
- [x] Add summary test: no API key returns deterministic fallback
- [x] Add summary test: attempted transient failure retries and succeeds
- [x] Add summary test: attempted timeout returns error, not fallback
- [x] Add summary tests for `Length` and `ContentFilter` finish reasons
- [x] Add collect test: summary failure writes no leaf and no index entry

## 7. Upstream title extraction quality

- [x] Add unit test: good metadata title is preserved when body has a different heading
- [x] Add unit test: empty metadata title uses first meaningful leading `#` heading
- [x] Add unit test: clearly chrome-like metadata title uses first meaningful leading `#` heading
- [x] Add unit test: clearly chrome-like metadata title uses first meaningful leading `##` heading when no `#` is available
- [x] Add unit test: deeper `###` heading is ignored for title selection
- [x] Add unit test: no confident heading keeps existing extracted title
- [x] Implement `choose_title(metadata_title, body_markdown)` in `src/engine/extract.rs`
- [x] Implement first meaningful `#`/`##` heading extraction helper
- [x] Implement narrow chrome-like title predicate without broad source-specific guessing
- [x] Use selected title for duplicate leading H1 stripping in `extract_content`
- [x] Add deterministic mdBook/Rust Book-like fixture where polluted title would be `Keyboard shortcuts`
- [x] Add collect-level regression test expecting title `Understanding Ownership`
- [x] Verify collected filename/slug uses `understanding-ownership`
- [x] Verify leaf frontmatter title and `index.jsonl` title are `Understanding Ownership`

## 8. Final verification

- [x] Run `cargo fmt --check`
- [x] Run `cargo clippy --all-targets --all-features -- -D warnings`
- [x] Run `cargo test`
- [x] Confirm `Cargo.toml` has no direct `regex` dependency
- [x] Run `cargo tree -i regex` and confirm remaining regex use, if any, is transitive only
- [x] Live smoke: rebuild `tmp-tree` from `corpora/default/urls.txt` with `bo collect`
- [x] Live smoke: run `bo query` against rebuilt `tmp-tree` and inspect answer, citations, and source titles
