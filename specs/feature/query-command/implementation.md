# bo query V1 — Implementation Summary

## What Shipped

`bo query <question>` command that answers natural-language questions using the local knowledge base, returning LLM-synthesized answers with `[[wikilink]]` citations pointing back to source leaves.

## Architecture

Deterministic pipeline — no loops, no retries, no agentic behaviour:

```
extract terms → retrieve leaves → assemble context → single structured-output LLM call → validate citations → format output
```

## Key Files

| File | Role |
|------|------|
| `src/cli/query.rs` | Core module (~480 LOC) |
| `src/main.rs` | CLI wiring |
| `src/engine/config.rs` | `query_model` field |
| `src/tests/cli_query_tests.rs` | Unit tests |
| `tests/integration_query.rs` | Integration tests with mock provider |

## Design Decisions

- **Self-contained module** — no code sharing with `bo search`. Different semantics (OR vs AND), different evolution paths. Duplication is intentional.
- **Stop-word extraction** with possessive stripping (`'s`, `'t`) and min-length filter (3 chars).
- **OR-semantics density scoring** over all leaves (read via `index.jsonl`).
- **Two-tier context assembly:**
  - Top-10 leaves get slug/title/url/summary (breadth)
  - Top-5 leaves get full body (depth)
  - Capped at ~60k words
- **Single structured-output LLM call** via existing `LlmProvider` trait / `OpenAiProvider`.
- **Citation validation:** strips invalid `[[slug]]` from both `cited_slugs` list AND answer prose via regex.
- **Hard failure on missing API key** — exits with code 2. No silent degradation to search.
- **Separate `query_model` config field**, defaults to `gpt-4o`.
- **`run_with_provider()`** for injectable provider in tests.

## Test Coverage

- **16 unit tests** — extraction, retrieval, assembly, validation
- **6 integration tests** — mock provider, error paths, boundary cases
- **1 ignored live API test** — requires real key, not run in CI

## Dependencies Added

- `regex` crate

## Dogfood Results

Tested against the 54-leaf default dogfood corpus. Produces coherent answers with valid citations.

**Known quality issue:** inherited from upstream title extraction — Rust Book pages are titled "Keyboard shortcuts" because the scraper picks up page chrome rather than the actual heading.

## ADR Compliance

| ADR | Status |
|-----|--------|
| ADR-001 (deterministic pipeline) | ✅ Compliant |
| ADR-002 (JSON schema) | ✅ Compliant |
| ADR-003 (V1 prompt-chaining, V2 agentic deferred) | ✅ V1 ships single-call; agentic path deferred |

## Commits

6 commits on `feature/query-command` branch.
