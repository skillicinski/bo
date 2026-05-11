# Implementation: Query V1 operational hardening

## Status

Complete. All tasks in `tasks.md` are checked off.

## Summary of changes

### Query hardening

- Replaced regex-based invalid wikilink cleanup with a deterministic parser in `src/cli/query.rs`.
- Final citation metadata now comes from the union of:
  1. valid wikilinks found in answer prose, in first-appearance order;
  2. remaining valid `cited_slugs` returned by the model.
- Invalid well-formed `[[slug]]` links are unwrapped to plain text; malformed/nested/empty/unclosed bracket text is left unchanged.
- Removed direct `regex` dependency from `Cargo.toml`; remaining regex usage is transitive through `trafilatura` dependencies.

### Query context budgeting

- Added model context lookup for:
  - `gpt-4o` — 128k
  - `gpt-4o-mini` — 128k
  - `gpt-4.1` — 1M
  - `gpt-4.1-mini` — 1M
  - `gpt-4.1-nano` — 1M
- Replaced fixed `60_000` word budget with computed source word budget.
- Unknown `query_model` now fails before retrieval/provider invocation with `QueryError::UnknownModelContext`.
- Too-small context windows fail with `QueryError::ContextBudgetExhausted`.
- Added JSON error mappings: `unknown_model_context`, `context_budget_exhausted`.

### Shared LLM policy

- Added `LlmCallPolicy` and `complete_with_policy(...)` in `src/engine/llm/mod.rs`.
- Added bounded timeout/retry handling with explicit policies:
  - query synthesis: 60s/attempt, 3 total attempts;
  - leaf summary: 30s/attempt, 3 total attempts;
  - compile synthesis: 180s/attempt, 3 total attempts.
- Added typed transient/permanent error categories for retry classification.
- Added fake-provider tests for retry success, retry exhaustion, timeout, and permanent no-retry behavior.
- Enabled `tokio/time`.
- Added direct `backoff` dependency to control `async-openai` internal retry behavior.
- Configured `OpenAiProvider` to avoid hidden long retry loops so bo’s policy is authoritative.

### Query synthesis

- Query synthesis now calls `complete_with_policy(...)`.
- `FinishReason::Length` returns `QueryError::Truncated` before parsing partial content.
- `FinishReason::ContentFilter` returns `QueryError::ContentFilter` before parsing blocked content.
- Added tests for transient retry, timeout, length, and content-filter behavior.

### Compile synthesis

- Compile now calls `complete_with_policy(...)` through an injectable helper.
- Preserved existing compile errors:
  - `CompileError::ContextOverflow`
  - `CompileError::Truncated`
  - `CompileError::ContentFilter`
- Added fake-provider tests for transient retry, timeout, permanent no-retry, length, and content-filter behavior.

### Summary generation and collection

- Added `SummaryError`.
- Summary LLM generation now calls `complete_with_policy(...)`.
- Missing `OPENAI_API_KEY` still uses deterministic first-200-words fallback.
- If a provider call is attempted, timeout/retry exhaustion/parse/truncation/content-filter failures now return an error instead of falling back.
- `CollectError::Summary` surfaces summary failures.
- Collection aborts before leaf/index writes when attempted summary generation fails.
- `collect --json` maps summary failures to `llm_error`.

### Title extraction quality

- Added selected-title logic in `src/engine/extract.rs`.
- Good metadata titles are preserved.
- Empty or clearly chrome-like metadata titles can be replaced with the first meaningful leading `#` or `##` heading.
- Deeper `###` headings are ignored.
- V1 chrome predicate is intentionally narrow: exact normalized `Keyboard shortcuts`.
- Duplicate leading H1 stripping now uses the selected title.
- Added mdBook/Rust Book fixture coverage verifying future collection writes `Understanding Ownership` into:
  - slug/filename;
  - leaf frontmatter title;
  - `index.jsonl` title.

## Verification

Ran successfully:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
cargo tree -i regex
```

`cargo tree -i regex` still reports transitive regex users through `trafilatura`; `Cargo.toml` has no direct `regex` dependency.

## Live smoke

Rebuilt `tmp-tree` from `corpora/default/urls.txt`:

- 54 collected successfully
- 5 failed/rejected during collection

Ran query smoke against rebuilt `tmp-tree` after sourcing the local `.env`:

```bash
source .env
HOME=<tmp-home> target/debug/bo query "what does Rust ownership do?"
```

Result: successful answer with citations including:

- `[[understanding-ownership]] — Understanding Ownership`
- `[[fearless-concurrency]] — Fearless Concurrency`

## Operational note: `.env`

The local `.env` uses shell syntax with command substitution:

```sh
export OPENAI_API_KEY=$(...)
```

`dotenvy::dotenv()` does not execute command substitutions, so running the binary directly does not load that key. For local manual smoke tests, source the file first:

```bash
source .env
bo query "..."
```

## Files changed

- `Cargo.toml`
- `Cargo.lock`
- `src/cli/collect.rs`
- `src/cli/compile.rs`
- `src/cli/query.rs`
- `src/engine/extract.rs`
- `src/engine/llm/mod.rs`
- `src/engine/llm/providers/openai.rs`
- `src/engine/summary.rs`
- `src/main.rs`
- `src/tests/cli_collect_tests.rs`
- `src/tests/cli_compile_tests.rs`
- `src/tests/cli_query_tests.rs`
- `src/tests/engine_extract_tests.rs`
- `src/tests/engine_llm_tests.rs`
- `src/tests/engine_summary_tests.rs`
- `specs/chore/query-v1-operational-hardening/tasks.md`
