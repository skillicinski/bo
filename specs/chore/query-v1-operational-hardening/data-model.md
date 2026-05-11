# Data model: Query V1 operational hardening

No persistent storage schema changes. Existing leaf frontmatter and `index.jsonl` remain authoritative; future collection runs write improved titles into the existing fields.

## Existing persisted structures touched

### Leaf frontmatter

Existing fields used by this chore:

```yaml
title: "Understanding Ownership"
url: "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html"
collected_at: "..."
updated_at: "..."
summary: "..."
```

Change: future collections should write the selected content-specific title into `title`. No migration of existing leaves.

### `index.jsonl`

Existing shape:

```json
{"file":"understanding-ownership.md","title":"Understanding Ownership","url":"https://..."}
```

Change: future collections should use the same selected title for slug/index generation. No index rebuild/backfill.

## New runtime structures

### `SelectedTitle`

Implementation can use a plain `Option<String>`, but tests are easier if selection reason is representable internally.

```rust
struct SelectedTitle {
    value: String,
    source: TitleSource,
}

enum TitleSource {
    Metadata,
    BodyHeading,
}
```

Public API does not need to expose this. If not implemented as a struct, equivalent behavior should still be test-covered.

### `LlmCallPolicy`

Provider-independent operational policy.

```rust
pub struct LlmCallPolicy {
    pub timeout: std::time::Duration,
    pub max_attempts: usize,
    pub initial_backoff: std::time::Duration,
}
```

Relationships:

- consumed by `engine::llm::complete_with_policy`;
- defined by query, summary, and compile callers with explicit constants;
- tests can inject millisecond-scale policies.

Required caller constants:

```rust
const QUERY_LLM_POLICY: LlmCallPolicy = ...;   // 60s, 3 attempts
const SUMMARY_LLM_POLICY: LlmCallPolicy = ...; // 30s, 3 attempts
const COMPILE_LLM_POLICY: LlmCallPolicy = ...; // 180s, 3 attempts
```

### `LlmError`

Extend existing enum with typed operational failures so callers avoid string matching.

Current shape:

```rust
pub enum LlmError {
    Network(String),
    Api(String),
    Parse(String),
}
```

Target shape can be equivalent to:

```rust
pub enum LlmError {
    Network(String),
    Api(String),
    Parse(String),
    Timeout { timeout: Duration },
    RetryExhausted { attempts: usize, last_error: Box<LlmError> },
    Truncated,
    ContentFilter,
}
```

If Rust recursion ergonomics require a different representation, preserve these semantic categories.

### `ModelContextWindow`

Known model metadata.

```rust
pub struct ModelContextWindow {
    pub model: &'static str,
    pub context_tokens: usize,
}
```

Initial table:

| Model | `context_tokens` |
| --- | ---: |
| `gpt-4o` | `128_000` |
| `gpt-4o-mini` | `128_000` |
| `gpt-4.1` | `1_000_000` |
| `gpt-4.1-mini` | `1_000_000` |
| `gpt-4.1-nano` | `1_000_000` |

Lookup:

```rust
pub fn context_window_tokens(model: &str) -> Option<usize>;
```

Unknown models return `None` and cause query to fail before provider calls.

### `QueryContextBudget`

Computed prompt/source allocation for query.

```rust
pub struct QueryContextBudget {
    pub model: String,
    pub context_tokens: usize,
    pub reserved_tokens: usize,
    pub source_tokens: usize,
    pub source_words: usize,
}
```

Relationships:

- constructed from `query_model`, completion token cap, and prompt overhead reserve;
- passed into context assembly or reduced to `source_words`;
- failure maps to query errors before retrieval/provider call if unknown or too small.

### `QueryError` additions

Extend `src/cli/query.rs` errors with variants equivalent to:

```rust
pub enum QueryError {
    UnknownModelContext { model: String },
    ContextBudgetExhausted {
        model: String,
        context_tokens: usize,
        reserved_tokens: usize,
    },
    Truncated,
    ContentFilter,
    // existing variants...
}
```

Exit code: 2.

JSON mapping in `main.rs`:

- `UnknownModelContext` -> `unknown_model_context`
- `ContextBudgetExhausted` -> `context_budget_exhausted`
- `Truncated` / `ContentFilter` -> `llm_error` or more specific existing-compatible codes if desired

### `SynthesisResponse`

Existing structured LLM response:

```rust
struct SynthesisResponse {
    answer: String,
    cited_slugs: Vec<String>,
}
```

No schema change. Validation changes how final citations are derived from this plus answer prose.

### `CitationValidationResult`

Implementation can return the existing tuple, but behavior should match:

```rust
struct CitationValidationResult {
    answer: String,
    citations: Vec<Citation>,
}
```

Construction rules:

1. sanitize invalid well-formed wikilinks in `answer`;
2. collect valid prose wikilinks in first-appearance order;
3. append valid structured `cited_slugs` not already seen;
4. map slugs to existing `Citation` metadata.

### `SummaryError`

New error type for fallible attempted LLM summaries.

```rust
pub enum SummaryError {
    Llm(LlmError),
    Parse(String),
    Truncated,
    ContentFilter,
}
```

Relationships:

- `summary::generate(...) -> Result<String, SummaryError>` or equivalent;
- no API key returns `Ok(generate_fallback(...))`;
- attempted provider failures return `Err(...)`;
- `CollectError` adds a summary variant to surface this.
