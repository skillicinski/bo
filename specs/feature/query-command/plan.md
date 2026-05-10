# Plan: `bo query` V1

## Architecture

### Pipeline

```
user question
  → term extraction (deterministic, stop-word removal)
  → leaf retrieval (OR-semantics density scoring over all leaves)
  → context assembly (summaries for breadth, full bodies for depth)
  → single structured-output LLM call (synthesis + citations)
  → output formatting (human/JSON)
```

This is a deterministic pipeline with one LLM call, consistent with ADR-001. The retrieval is entirely independent of `bo search` — different semantics (OR vs AND), different scoring needs, different output.

### Key components

| Component | Location | Responsibility |
|-----------|----------|---------------|
| `cli::query` | `src/cli/query.rs` | Orchestrates the pipeline: extract → retrieve → assemble → call → format |
| `cli::query::extract` | inline or small fn | Stop-word removal, term extraction from natural-language question |
| `cli::query::retrieve` | inline | Read all leaves, score by term density (OR semantics), rank, return top-k |
| `cli::query::assemble` | inline | Build LLM context: summaries of top-10, full bodies of top-5, within token budget |
| `cli::query::synthesize` | inline | Construct messages, call LlmProvider, parse structured response |
| Config extension | `src/engine/config.rs` | Add `query_model` field |
| CLI wiring | `src/main.rs` | Add `Query` variant to `Commands` enum, dispatch |

### Rationale: no code sharing with search

`bo search` is a user-facing command with AND semantics, pagination, KWIC snippets, and specific UX. Query's retrieval is an internal step with OR semantics, no pagination, and scores leaves for LLM context assembly. Sharing code would couple two features evolving in different directions (search stays deterministic/strict; query retrieval gets replaced by agentic navigation in V2).

### Integration points

- **`domain::index`** — read `index.jsonl` to enumerate leaves (file paths + titles)
- **`domain::frontmatter`** — parse leaf frontmatter for summary, title, url
- **`engine::llm::LlmProvider`** / `OpenAiProvider` — synthesis call with structured output
- **`engine::config`** — read `query_model` from config
- **`cli::json`** — JSON output utilities if applicable

### External dependencies

- OpenAI API (existing `OPENAI_API_KEY` env var, existing `OpenAiProvider`)
- No new crate dependencies anticipated

## Implementation strategy

### Order of work

1. **Config**: add `query_model: Option<String>` to `Config`, add `effective_query_model()` accessor
2. **Core module**: `src/cli/query.rs` with the pipeline functions
3. **CLI wiring**: add `Query` subcommand to main, dispatch to `cli::query`
4. **Tests**: unit tests for term extraction, retrieval scoring, context assembly; integration test with mocked LLM response

### Term extraction

Aggressive stop-word list covering question words and common articles/prepositions:

```
what, which, who, whom, where, when, why, how, is, are, was, were, am,
do, does, did, has, have, had, can, could, would, should, will, shall,
the, a, an, of, in, on, at, to, for, with, by, from, about, between,
and, or, but, not, no, if, then, than, that, this, these, those, it, its
```

Input: `"what are the tradeoffs of Rust's ownership model?"`
After extraction: `["tradeoffs", "rust's", "ownership", "model"]`

Punctuation stripped from term boundaries. Terms lowercased.

### Retrieval

Read every leaf file in the tree. For each:
1. Parse frontmatter → extract title, url, summary
2. Concatenate title + summary + body → lowercase
3. Score: count occurrences of each term, sum, normalize by word count (density)
4. Filter: require at least one term to match (score > 0)
5. Sort by density descending
6. Return top-10 with metadata + summary + body

No index structure needed beyond `index.jsonl` for file enumeration. Full file reads are acceptable — V1 targets trees of <200 leaves where this completes in <100ms.

### Context assembly

Two tiers within the LLM prompt:

- **Breadth tier** (top 10): slug, title, url, summary — lets the model know what's available
- **Depth tier** (top 5): full body content — gives the model material to synthesize from

Token budget heuristic: cap total assembled context at ~60,000 words (~80k tokens). If top-5 full bodies exceed budget, truncate longest bodies or reduce to top-3. Word count is sufficient for V1 — no tiktoken dependency.

### Synthesis prompt

System prompt instructs:
- Answer the question using ONLY the provided source material
- Cite sources using `[[slug]]` wikilink format inline in the prose
- If sources don't contain enough information to answer, say so explicitly
- Do not invent information not present in the sources
- Keep answer concise (1–3 paragraphs)

Structured output schema:
```json
{
  "type": "object",
  "properties": {
    "answer": { "type": "string", "description": "Prose answer with [[slug]] citations inline" },
    "cited_slugs": { "type": "array", "items": { "type": "string" }, "description": "List of leaf slugs actually cited in the answer" }
  },
  "required": ["answer", "cited_slugs"],
  "additionalProperties": false
}
```

### Citation validation

After receiving the LLM response:
1. Parse `cited_slugs` from structured output
2. Verify each slug corresponds to a leaf file that was in the retrieval set
3. Strip any hallucinated citations that don't map to real leaves
4. Build citation metadata (slug, title, file) for JSON output

### Output formatting

**Human mode** (default):
```
<prose answer with [[slug]] wikilinks>

Sources:
  • [[leaf-slug]] — Leaf Title
  • [[another-slug]] — Another Title
```

**JSON mode** (`--json`):
```json
{
  "answer": "...",
  "citations": [
    { "slug": "leaf-slug", "title": "Leaf Title", "file": "leaves/leaf-slug.md" }
  ],
  "model": "gpt-4o",
  "leaves_consulted": 5
}
```

### Error handling

| Condition | Behaviour |
|-----------|-----------|
| No `OPENAI_API_KEY` and no provider configured | Exit 2, message: "No API key configured. Set OPENAI_API_KEY or configure a provider." |
| Tree not seeded | Exit 2, standard not-seeded message |
| No leaves in tree | Exit 1, "no sources collected yet" |
| Retrieval returns 0 matches | Exit 1, "no relevant sources found in tree" |
| LLM call fails (network, auth, rate limit) | Exit 2, surface provider error message |
| LLM response fails schema parse | Exit 2, "synthesis failed — invalid response from model" |
