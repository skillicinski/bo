# Spec: Leaf Summary Field

## Problem statement

Large collections (50+ docs, >128K tokens) overflow the compile model's context window. Downstream features (`bo query`, compile overflow recovery) need a compressed representation of each leaf to triage relevance without reading full bodies. Currently no such representation exists — the only options are full content or the title alone.

## User-facing requirements

1. **Summary generated at collect time**: `bo collect <url>` produces a `summary:` field in the leaf's YAML frontmatter alongside existing fields (title, url, collected_at, updated_at).
2. **LLM-powered summary (when available)**: if an LLM provider is configured (API key present), generate a ~200-word prose summary via a single structured-output call. The summary captures what the document is about — key topics, main argument, distinctive content — optimized for retrieval triage.
3. **Deterministic fallback (no API key)**: if no LLM provider is configured, generate the summary by extracting the first ~200 words of body content. The leaf is still written successfully — summarization never blocks collection.
4. **Summary stored in frontmatter**: persisted as `summary:` field in the leaf's YAML frontmatter. Available to all downstream consumers (search, query, compile) without re-generation.
5. **Idempotent**: if a leaf already has a `summary:` field, `bo collect` does not overwrite it (collect is deduplicated by URL anyway, but the contract is clear).
6. **Provider config reuse**: uses the same LLM provider configuration as compile (existing config). No new config fields required for MVP.

## Success criteria

- Every newly collected leaf has a `summary:` field in frontmatter after `bo collect` completes.
- With an API key configured: summary is a coherent ~200-word prose paragraph describing the document's content, suitable for retrieval triage.
- Without an API key: summary is the first ~200 words of the extracted body, cleanly truncated at a word boundary.
- Collect latency increase is acceptable: one cheap/fast model call per URL (not blocking on expensive models).
- Existing leaves without summaries continue to function — no breaking change to commands that read frontmatter.
- Summary field is valid YAML (properly quoted/escaped for multi-line or special-character content).

## Out of scope

- Backfill command for existing leaves (future `bo summarize` or similar).
- Compile using summaries as overflow fallback (separate feature, consumes `summary:` field).
- Search weighting summaries differently from body content (separate feature).
- `bo query` using summaries for pre-filtering (separate feature, consumes `summary:` field).
- Separate `summary_model` config field (future improvement — noted in features.md under `bo config set`).
- Summary regeneration or update on re-collect.

## Open questions

None — resolved during spec discussion.
