# Analysis: Query V1 operational hardening

## Risk assessment

**Moderate-to-high implementation risk**. The intended behavior is clear enough, but the LLM policy work crosses async runtime behavior, provider internals, CLI error mapping, and tests that manipulate environment variables.

### Main risks

1. **Hidden provider retries can violate the visible 3-attempt policy**
   - `async-openai` already retries 429/5xx internally via `backoff::ExponentialBackoff`.
   - If this remains enabled, `complete_with_policy` may perform 3 wrapper attempts while each attempt performs hidden provider retries.
   - Mitigation: disable/tightly bound `async-openai` backoff or implement the OpenAI chat completion call directly enough to preserve HTTP status and attempt count.

2. **`tokio::time` is not currently enabled**
   - `Cargo.toml` currently enables `tokio` features `rt` and `macros`; timeout/sleep require the `time` feature.
   - Mitigation: add `tokio` `time` feature as an explicit prerequisite task before implementing `complete_with_policy`.

3. **Provider error classification may be lossy**
   - Current `OpenAiProvider` maps all API failures to `LlmError::Api(String)` and loses HTTP status.
   - Spec requires different handling for 429/5xx/network vs auth/quota/non-429 4xx.
   - Mitigation: change provider error mapping to typed categories. Avoid broad string matching except as a temporary compatibility shim for context-overflow messages.

4. **Summary behavior change can surprise collection flows**
   - Existing `summary::generate` falls back on any LLM failure; new behavior fails collection if a provider was attempted.
   - Tests and developer machines with `OPENAI_API_KEY` set may unexpectedly try live summary calls.
   - Mitigation: make summary provider injection possible for tests, use `serial_test` or env guards for env-dependent tests, and assert no leaf/index writes on attempted-provider failure.

5. **Environment-variable tests can race**
   - Existing compile/summary tests manipulate `OPENAI_API_KEY`. Cargo tests run in parallel by default.
   - Mitigation: use `serial_test` for all tests that set/remove provider env vars, or refactor tested functions to avoid env dependence.

6. **Query error precedence may change user-visible behavior**
   - Plan says unknown model/context budget should fail before retrieval. That means an empty/no-result tree with an unknown `query_model` exits 2 instead of preserving no-results/empty-tree exit 1.
   - Mitigation: decide and encode precedence in tests. Prefer early config validation if operator misconfiguration should dominate tree state; otherwise validate after retrieval but before provider call.

7. **Compile context-overflow behavior can regress**
   - Current compile maps OpenAI text containing `maximum context length` to `CompileError::ContextOverflow`.
   - Typed provider errors may remove or alter that string path.
   - Mitigation: add a `LlmError::ContextOverflow` or preserve a narrowly tested compatibility path.

8. **Citation parser can silently corrupt prose if too aggressive**
   - Nested/malformed markdown-like text is common in LLM output.
   - Mitigation: only transform simple well-formed `[[non-empty-no-bracket-chars]]` spans. Leave everything else unchanged.

9. **Title heuristic can regress good collections**
   - Replacing metadata title with body heading too broadly can create worse titles.
   - Mitigation: keep the suspicious-title predicate narrow; preserve metadata unless missing or clearly chrome-like. Test good-title preservation.

10. **Timeout retries can duplicate provider work/cost**
    - Dropping a timed-out future does not guarantee the upstream provider aborted generation.
    - Mitigation: acceptable for V1 because calls are read-only, but error messages should not imply no provider work occurred.

## Gap analysis

1. **Exact query budget constants are still suggested, not final**
   - Plan suggests `QUERY_MAX_COMPLETION_TOKENS = 2048`, `QUERY_PROMPT_OVERHEAD_TOKENS = 4096`, `MIN_QUERY_SOURCE_WORDS = 1000`.
   - Implementation would otherwise choose ad hoc values.
   - Recommendation: make these constants final for V1 and test them.

2. **Token-to-word conversion formula is underspecified**
   - Spec says convert conservatively; plan mentions conservative conversion but not a fixed formula.
   - Recommendation: use `source_words = source_tokens * 3 / 4` or a stricter formula, then encode in tests.

3. **Unknown-model validation precedence is not fully specified**
   - Spec says fail before provider call; plan/tasks say before retrieval/provider call.
   - Recommendation: choose one. If keeping plan behavior, add tests showing unknown model beats empty/no-results retrieval.

4. **`clearly chrome-like` title predicate is intentionally vague**
   - Requirement avoids broad denylists, but implementation must decide what is chrome-like.
   - Recommendation: V1 predicate should be minimal: empty/whitespace and exact normalized `Keyboard shortcuts` are enough for the known mdBook issue. Add more only with fixtures.

5. **`meaningful heading` needs a precise parser rule**
   - Need to decide whether headings with links/emphasis/code are accepted, whether Setext headings count, and how much whitespace is allowed.
   - Recommendation: V1 accepts ATX headings only: leading optional whitespace, `# ` or `## `, non-empty stripped text, not `###`; strip basic markdown link syntax only if already available via existing helpers.

6. **Machine-readable errors are incomplete for collect summary failures**
   - Tasks mention `CollectError::Summary` display text, but spec requires provider failures to be structured for machine clients.
   - Recommendation: update `collect_json_error` to map summary failures to `llm_error` or `summary_error` with details.

7. **Finish-reason error ownership is ambiguous**
   - `FinishReason::Length`/`ContentFilter` are returned in `LlmResponse`, not provider errors.
   - Recommendation: callers should convert finish reasons to domain errors (`QueryError`, `CompileError`, `SummaryError`) rather than making provider `complete` fail on finish reason.

8. **Fake-provider injection points are not all present**
   - Query has `run_with_provider`; compile and summary mostly instantiate providers internally.
   - Recommendation: add small private provider-injectable helpers for compile and summary rather than testing through full CLI/env paths.

9. **OpenAI-compatible/local models are out of scope but affected**
   - Unknown model hard failure means any local/custom model string cannot query.
   - This is intended by spec, but error text must say the model context window is unknown and instruct the operator to choose/add a known model.

10. **Analysis artifact was stale before this update**
    - Previous analysis still listed query module split and retrieval optimization as in-scope.
    - Current spec/plan/tasks correctly defer those. Implementation should follow spec/plan/tasks, not old triage notes.

## Edge cases

1. **Empty or no-result tree with invalid `query_model`**
   - Needs explicit expected error precedence.

2. **Question term extraction fails before model validation**
   - Existing `NoTerms` should probably remain first because it is pure user input validation.

3. **Model context exactly equals reserved tokens**
   - Should fail with context-budget exhausted; avoid underflow/saturating success with zero source words.

4. **Large known model with huge retrieved context**
   - Assembly should truncate to computed word budget and still return `leaves_consulted` consistently.

5. **LLM returns valid JSON with `FinishReason::Length`**
   - Must fail before parsing/using content.

6. **LLM returns invalid JSON after successful stop**
   - Parse error is permanent and should not be retried by policy. Query/summary/compile should surface parse/validation errors clearly.

7. **Fake timeout future continues after timeout**
   - Tests should count attempts at call start, not future completion.

8. **`max_attempts = 0` policy**
   - Not a production constant, but helper should reject or normalize invalid policy values to avoid infinite/zero-attempt ambiguity.

9. **Malformed wikilinks**
   - Examples: `[[`, `[[foo`, `[[]]`, `[[foo]`, `[[foo[[bar]]`, `[[foo]]bar[[baz]]`. Only simple well-formed non-empty spans should be transformed.

10. **Duplicate citations from answer and `cited_slugs`**
    - Deduplicate by slug, preserving first prose appearance then remaining structured order.

11. **Valid slug cited in prose but omitted from `cited_slugs`**
    - Must appear in final Sources/JSON citations.

12. **`cited_slugs` references valid slug not in prose**
    - Must appear after prose citations.

13. **`cited_slugs` contains duplicates**
    - Deduplicate without duplicating Sources rows.

14. **Title body starts with repeated chrome headings before document heading**
    - Current scope only guarantees first meaningful `#`/`##`. If mdBook extraction emits `# Keyboard shortcuts` before `# Understanding Ownership`, the V1 heuristic may fail unless the predicate skips chrome headings too.

15. **Title selection when metadata is `Keyboard shortcuts` but no heading exists**
    - Keep existing title rather than guessing.

16. **Summary collection failure after leaf path is computed**
    - Ensure summary is generated before `leaf::write` and `index::append_entry` to avoid partial artifacts.

17. **Live smoke with `corpora/default/urls.txt` can fail for network/provider reasons**
    - Treat it as a manual smoke, not a deterministic CI gate.

## Dependencies

1. **Cargo features/dependencies**
   - Add `tokio/time` feature.
   - Removing direct `regex` is expected.
   - Adding direct `backoff` may be justified if needed to control `async-openai` client backoff.

2. **`async-openai` internals**
   - Public errors may not expose HTTP status.
   - Internal retry behavior must be controlled or bypassed to satisfy the visible attempt policy.

3. **OpenAI model context limits**
   - Static table can drift. This is acceptable for V1 but should be isolated and easy to update.

4. **Environment**
   - `OPENAI_API_KEY` presence changes summary behavior.
   - Live smoke requires network and a valid API key for query.

5. **Transitive `regex`**
   - `trafilatura` uses regex internally; `cargo tree -i regex` will likely still show transitive use. Only direct dependency removal is required.

6. **Test runtime**
   - Async timeout tests must use injected millisecond policies; never use production 30/60/180 second constants.

## Recommendation

**Not ready to implement until two small plan/task adjustments are made.**

Address first:

1. Add a task to enable `tokio`'s `time` feature.
2. Add tasks/tests for collect `--json` summary failure mapping.
3. Decide query validation precedence for unknown model vs empty/no-result tree and encode it in tests.
4. Finalize query budget constants and token-to-word conversion formula.
5. Decide the concrete V1 chrome-title predicate, preferably minimal: empty/whitespace plus exact normalized `Keyboard shortcuts` until more fixtures exist.
6. Decide how to control `async-openai` hidden retry/backoff before coding the policy wrapper.

After those are resolved, implementation can proceed. The highest-risk slice is shared LLM policy/provider classification; do that behind fake-provider tests before touching query/compile/summary callers.
