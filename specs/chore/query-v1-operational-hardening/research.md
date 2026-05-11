# Research: Query V1 operational hardening

## 1. Title extraction with `trafilatura`

### Finding

The Rust crate `trafilatura 0.3.0` already extracts metadata titles from multiple sources internally:

- standard metadata selectors such as `citation_title`, `dc.title`, `headline`, `twitter:title`;
- OpenGraph `og:title`;
- JSON-LD article/name/headline fields;
- DOM `<title>` fallback when metadata title is absent.

Its public `Options` expose extraction focus, fallback, links/images, pruning, language/date behavior, and metadata requirements, but no obvious alternate title-ranking API or “prefer article heading over DOM title” option.

### Decision

Do not add a new title extraction dependency in this chore. Keep `trafilatura` as the extractor and add a narrow post-processing selector in `engine::extract`:

1. keep good metadata titles;
2. if metadata title is missing or clearly chrome-like, use first meaningful leading `#`/`##` from extracted markdown;
3. otherwise keep existing metadata title.

### Validation needed during implementation

- Build an offline mdBook/Rust Book-like fixture that reproduces polluted `Keyboard shortcuts` metadata.
- Confirm extracted markdown contains `# Understanding Ownership` or `## Understanding Ownership` in a stable position.
- Confirm good metadata titles remain unchanged when body headings differ.

## 2. `async-openai` internal retry/backoff

### Finding

`async-openai 0.34.0` already performs internal retry/backoff for server errors and 429 rate limits in the client execution path. It uses a `backoff::ExponentialBackoff` stored inside `Client`, and `Client::with_config(...)` uses the default backoff.

This conflicts with the spec's requirement that bo's policy be visible and bounded at 3 total attempts. If left unchanged, a single `provider.complete(...)` call may itself perform hidden retries before bo's wrapper retry policy sees an error.

### Decision

Implementation must control or neutralize provider-internal retry behavior so bo's `LlmCallPolicy` is the authoritative policy.

Preferred options, in order:

1. Configure `async-openai::Client::build(...)` with an explicit `backoff::ExponentialBackoff` that disables or tightly bounds internal retries, then let `complete_with_policy` own retry attempts.
2. If disabling internal retry requires a direct `backoff` dependency, add it only for explicit OpenAI client configuration and document the reason in code.
3. If the crate cannot be cleanly constrained, replace only the OpenAI chat-completion request path with a small `reqwest` call that preserves HTTP status for classification.

### Validation needed during implementation

- Fake provider tests validate bo policy logic independent of OpenAI.
- OpenAI provider construction should be inspected/tested to ensure it does not retain long hidden retry loops.
- Error classification should not depend on parsing arbitrary display strings when structured status/category information is available.

## 3. Provider error classification

### Required categories

Retryable:

- timeout raised by bo policy;
- HTTP 429/rate limit;
- HTTP 5xx;
- transport/network failures.

Permanent:

- schema parse errors;
- validation errors;
- missing API key/config;
- auth failures;
- insufficient quota;
- non-429 HTTP 4xx;
- content filter;
- length/truncation.

### Finding

`async-openai::ApiError` exposes message/type/param/code but not the HTTP status after the client maps the response. Status is available inside `async-openai`'s private response handling but may not be preserved in the public error.

### Decision

Use typed `LlmError` variants for bo-owned failures and map provider failures as specifically as the client allows. If HTTP status is not available from `async-openai`, prefer adjusting the provider implementation over broad string matching.

## 4. Summary fallback semantics

### Finding

Current `summary::generate` falls back to deterministic first-200-words summary when:

- `OPENAI_API_KEY` is missing;
- runtime creation fails;
- LLM call fails;
- LLM response cannot be parsed.

The clarified spec changes only attempted-provider failure behavior.

### Decision

- Missing API key/no provider: keep deterministic fallback.
- Provider attempted: timeout/retry exhaustion/truncation/content-filter/parse failure is a hard collection failure.

### Implementation implication

`summary::generate` likely becomes fallible. `collect::write_new_document` must abort before writing leaf/index artifacts if attempted summary generation fails.

## 5. Query model context metadata

### Decision

Use a static table for currently supported/project-relevant OpenAI models:

| Model | Context window |
| --- | ---: |
| `gpt-4o` | 128k |
| `gpt-4o-mini` | 128k |
| `gpt-4.1` | 1M |
| `gpt-4.1-mini` | 1M |
| `gpt-4.1-nano` | 1M |

Unknown models fail before provider calls.

### Rationale

A conservative 8k fallback would be too small to be useful and could produce misleading behavior. Failing explicitly tells the operator to choose a known model or extend the table.

## 6. Regex removal

### Finding

The direct `regex` dependency is only needed for one query citation cleanup operation. Other crates may still use regex transitively, including `trafilatura`.

### Decision

Replace direct use with a small parser over characters/bytes for `[[...]]` spans.

Success condition:

- `Cargo.toml` has no direct `regex` entry;
- `cargo tree -i regex` may still show transitive users.

## 7. Performance considerations

No retrieval I/O optimization is in scope. Existing full-scan retrieval remains acceptable for V1 operational hardening. Model-aware context assembly may reduce prompt size, but it is a correctness guard rather than a performance feature.
