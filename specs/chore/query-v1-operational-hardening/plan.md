# Plan: Query V1 operational hardening

## Architecture decisions

1. **Keep `src/cli/query.rs` intact** — no module split in this chore. The command is still small enough; splitting is deferred until query grows into a larger subsystem.
2. **Do not change retrieval semantics** — no two-phase retrieval, threshold tuning, or large-tree I/O optimization. Existing retrieval/ranking tests should remain behavior-preserving guardrails.
3. **Shared LLM policy belongs in `engine::llm`** — timeout/retry behavior is provider-agnostic. Providers perform a single raw completion attempt; policy orchestration wraps providers.
4. **Operational constants stay visible** — query/summary/compile timeout and attempt constants should be named near their callers or in an obvious `engine::llm` policy section, not hidden in provider internals.
5. **Unknown model context is a hard query error** — do not fall back to an 8k budget. If bo cannot know the configured model's context window, it must fail before spending API calls.
6. **Title improvement is future-only extraction quality** — improve titles written by future `bo collect` runs. Do not mutate or repair existing leaves/index entries.
7. **Attempted LLM summary failures are hard failures** — deterministic summary fallback remains only for the no-provider/no-API-key path. If a provider call is attempted, timeout/retry exhaustion/truncation/content-filter/parse errors surface to the caller.

## Key components and responsibilities

### `src/engine/extract.rs`

Add deterministic title selection after `trafilatura` extraction and before leaf writing.

Responsibilities:

- inspect `trafilatura` metadata title and extracted markdown body;
- preserve good metadata titles;
- when title is missing or clearly chrome-like, select the first meaningful leading `#` or `##` heading;
- ignore deeper headings;
- keep the existing title if no confident better title exists;
- strip a leading duplicate H1 using the selected title, not necessarily the raw metadata title.

Candidate helpers:

```rust
fn choose_title(metadata_title: Option<&str>, body_markdown: &str) -> Option<String>;
fn first_meaningful_heading(body_markdown: &str) -> Option<String>;
fn is_clearly_chrome_title(title: &str) -> bool;
```

The suspicious-title heuristic should remain narrow. Prefer evidence from real metadata/headings over broad denylist behavior.

### `src/engine/llm/mod.rs`

Add provider-independent call policy.

Responsibilities:

- represent timeout/attempt/backoff policy;
- run `provider.complete(...)` under `tokio::time::timeout`;
- retry only transient failures;
- expose errors without string parsing in callers;
- provide known model context metadata for query budgeting.

Candidate API:

```rust
pub struct LlmCallPolicy {
    pub timeout: Duration,
    pub max_attempts: usize,
    pub initial_backoff: Duration,
}

pub async fn complete_with_policy(
    provider: &dyn LlmProvider,
    messages: &[Message],
    model: &str,
    max_tokens: u32,
    response_schema: Option<&serde_json::Value>,
    policy: LlmCallPolicy,
) -> Result<LlmResponse, LlmError>;

pub fn context_window_tokens(model: &str) -> Option<usize>;
```

Extend `LlmError` with typed variants as needed, e.g. timeout, retry exhaustion, transient API/network error, truncation, content filter, unknown model context. Avoid callers matching arbitrary provider strings except for legacy context-overflow compatibility.

Known model context table:

| Model | Context window |
| --- | ---: |
| `gpt-4o` | 128k tokens |
| `gpt-4o-mini` | 128k tokens |
| `gpt-4.1` | 1M tokens |
| `gpt-4.1-mini` | 1M tokens |
| `gpt-4.1-nano` | 1M tokens |

### `src/engine/llm/providers/openai.rs`

Provider should map OpenAI/transport errors into typed `LlmError` variants sufficiently for policy retry decisions.

Responsibilities:

- keep request construction and raw API call logic;
- return `FinishReason` faithfully;
- classify network/transport failures as transient;
- classify rate limit and 5xx as transient where status information is available;
- classify request-building, schema, auth, quota, and other 4xx failures as permanent.

Implementation must account for `async-openai`'s own internal retry/backoff. The target behavior is a visible bo policy of 3 total provider attempts; do not accidentally allow hidden long retry loops inside the OpenAI client.

### `src/engine/summary.rs`

Make LLM summary generation explicitly fallible when a provider call is attempted.

Responsibilities:

- no API key: preserve current deterministic `generate_fallback` behavior;
- API key present: call `generate_llm` through `complete_with_policy` using summary policy;
- provider timeout/retry exhaustion/truncation/content-filter/parse failure: return a summary error instead of silently falling back;
- keep deterministic fallback helper for no-provider mode and tests.

This likely changes the public orchestrator from `generate(...) -> String` to a fallible shape such as `generate(...) -> Result<String, SummaryError>`.

### `src/cli/collect.rs`

Propagate summary failures from collection.

Responsibilities:

- add `CollectError::Summary(summary::SummaryError)` or equivalent;
- keep missing-API-key summary fallback non-failing;
- when a summary provider is attempted and fails, abort collection before writing leaf/index artifacts.

### `src/cli/compile.rs`

Wrap compile LLM call in shared policy.

Responsibilities:

- define visible compile policy constant: 180s per attempt, 3 total attempts;
- call `complete_with_policy` from `call_llm`;
- treat `FinishReason::Length` as `CompileError::Truncated`;
- treat `FinishReason::ContentFilter` as `CompileError::ContentFilter`;
- preserve existing context-overflow behavior where OpenAI reports maximum context length.

### `src/cli/query.rs`

Keep file layout unchanged and harden internals.

Responsibilities:

- define visible query policy constant: 60s per attempt, 3 total attempts;
- compute context budget from `query_model` before assembling context;
- fail before provider call for unknown model context or too-small computed budget;
- call `complete_with_policy` for synthesis;
- treat `FinishReason::Length`/`ContentFilter` as query errors before parsing response;
- replace regex citation cleanup with deterministic parser;
- build final citation metadata from valid prose wikilinks union valid `cited_slugs`.

### `src/main.rs`

Update query JSON error mapping.

Responsibilities:

- map unknown-model/context-budget query failures to distinct JSON codes, e.g. `unknown_model_context` and `context_budget_exhausted`;
- keep these as exit code 2 provider/config-style failures.

## LLM policies

Use these constants:

| Caller | Timeout per attempt | Max attempts | Retries |
| --- | ---: | ---: | ---: |
| query synthesis | 60s | 3 | 2 |
| leaf summary | 30s | 3 | 2 |
| compile synthesis | 180s | 3 | 2 |

Backoff can be simple and deterministic; use short initial delays and cap them so tests can override policies with millisecond durations.

Retry only:

- timeout;
- HTTP 429/rate limit;
- HTTP 5xx;
- clear network/transport failures.

Do not retry:

- schema parse errors;
- validation errors;
- content filter;
- truncation/length finish;
- missing API key;
- local configuration errors;
- auth/quota/non-429 4xx errors.

## Query context budget strategy

1. Resolve `query_model` to a known context window.
2. If unknown, return `QueryError::UnknownModelContext { model }` before retrieval/provider call.
3. Reserve completion tokens and prompt overhead.
4. Convert remaining source tokens to words conservatively.
5. If below minimum viable source budget, return `QueryError::ContextBudgetExhausted { ... }` before provider call.
6. Pass the computed word budget into `assemble_context`; remove use of the fixed `TOKEN_BUDGET_WORDS` constant.

Suggested constants can be selected during implementation, but they must be explicit and tested:

```rust
const QUERY_MAX_COMPLETION_TOKENS: usize = 2048;
const QUERY_PROMPT_OVERHEAD_TOKENS: usize = 4096;
const MIN_QUERY_SOURCE_WORDS: usize = 1000;
```

## Citation strategy

Parser behavior:

- valid well-formed `[[slug]]` where `slug` exists in the retrieval set: preserve exactly;
- invalid well-formed `[[slug]]`: replace with `slug`;
- malformed, nested, empty, or unclosed bracket text: leave unchanged.

Citation metadata order:

1. valid wikilinks found in final answer prose, in first-appearance order;
2. remaining valid `cited_slugs` returned by the model, in returned order;
3. dedupe by slug.

Remove direct `regex` dependency from `Cargo.toml` once no direct use remains.

## Integration points and dependencies

- `trafilatura` remains the extraction engine; no new title extraction dependency planned.
- `tokio::time` provides timeout/sleep for LLM policy.
- `async-openai` remains the OpenAI provider client, but its internal retry behavior must be audited/controlled.
- `regex` direct dependency should be removed; transitive `regex` via other crates is acceptable.
- No config file schema changes.
- No storage schema changes; improved titles flow through existing leaf frontmatter and `index.jsonl` fields.

## Implementation strategy

1. Add/adjust tests first for citation parser and model-budget failure cases.
2. Replace citation regex cleanup with parser and remove direct `regex` dependency.
3. Add model context table and update query budget assembly/error mapping.
4. Add shared LLM policy, typed errors, and fake-provider tests.
5. Update query synthesis to use shared policy and finish-reason errors.
6. Update compile synthesis to use shared policy and finish-reason errors.
7. Update summary generation to distinguish no-provider fallback from attempted-provider failure; propagate through collect.
8. Improve extraction title selection and add mdBook/Rust Book fixture tests.
9. Run verification.

## Verification commands

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo tree -i regex
```

`cargo tree -i regex` may still show transitive dependencies; success means `Cargo.toml` has no direct `regex` dependency.

## Risks and mitigations

- **Hidden provider retries**: `async-openai` has internal backoff. Mitigate by configuring it explicitly or replacing the raw request path so bo's visible policy controls attempts.
- **Summary behavior change**: collection can now fail after attempting LLM summary. Mitigate with clear errors and tests for no-key fallback vs attempted-provider failure.
- **Title heuristic overreach**: keep suspicious-title detection narrow; preserve metadata unless it is missing/clearly chrome-like.
- **Context-budget constants**: make reserves explicit and tested; unknown models fail rather than silently under-budgeting.
- **Async test duration**: tests must inject tiny policies and fake providers; never wait real 30/60/180s.
