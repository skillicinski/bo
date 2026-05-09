# Plan: `bo search` Implementation

## Architecture

### New module: `src/cli/search.rs`

Follows the same pattern as `list.rs`: a pure library function that accepts the tree directory and options, returns a typed result. No side effects, no mutation.

### Integration: `src/main.rs`

Add `Search` variant to the `Commands` enum with clap args. Wire to a `cmd_search` handler that calls `search::search_leaves()` and renders output.

### Module registration: `src/cli/mod.rs`

Add `pub mod search;`.

## Key components

| Component | Responsibility |
|-----------|---------------|
| `search_leaves(tree_dir, query, options)` | Orchestrates: read index → load leaves → match → score → sort → paginate |
| `parse_query(args)` | Splits CLI args into term list (each arg is one term/phrase) |
| `matches_all_terms(content_lower, terms_lower)` | AND gate: returns true only if every term appears as substring |
| `score_relevance(content_lower, terms_lower)` | `(sum of occurrences * 1000) / content.len()` — per-mille density |
| `extract_snippet(body, terms_lower, radius)` | Finds first term occurrence in body, returns ±80 char window. Collapses newlines to space for human output; JSON preserves raw. |
| `render_human(result)` | Formats title + snippet for terminal |
| `render_json(result)` | Serializes to JSON array |

## Implementation strategy

### Phase 1: Core matching (testable immediately)

1. `parse_query` — trivial, args are already split by the shell/clap. Each positional arg is one term or phrase.
2. `matches_all_terms` — lowercase both sides, `content.contains(term)` for each.
3. `score_relevance` — `(content.matches(term).count() summed * 1000) / content.len()`. Per-mille density normalization so short focused documents outscore long documents with incidental mentions.

### Phase 2: Snippet extraction

1. Find the byte offset of the first occurrence of any term in the lowercased body.
2. Map back to character boundaries (UTF-8 safe).
3. Extract `[offset - 80 .. offset + term.len() + 80]`, clamped to body bounds.
4. Prepend/append `…` if truncated on either side.
5. If no term found in body (match was in frontmatter only), return first 160 chars of body.

### Phase 3: Sorting and pagination

1. Default: sort by score descending, break ties by index position.
2. `--recent`: sort by `collected_at` descending (reuse `list.rs` date-parsing pattern).
3. Paginate: skip `(page - 1) * 5`, take 5.

### Phase 4: CLI integration

1. Add `Search` to `Commands` enum with positional `terms: Vec<String>`, `--page`, `--recent`, `--json` flags.
2. `cmd_search` calls `search::search_leaves`, renders, and returns appropriate exit code.

## Integration points

- **`domain::index::read_index`** — reused to enumerate leaf file paths.
- **`domain::frontmatter::parse`** — reused to split frontmatter from body and extract title/collected_at.
- **No new crates** — all matching is stdlib string operations.
- **Exit codes** — 0 (results), 1 (no results), 2 (usage/clap handles this).

## Risks and mitigations

| Risk | Mitigation |
|------|-----------|
| Large trees (500+ leaves) slow on full-file reads | Acceptable per spec (<500ms on 50+ leaves). Profile if needed; index-based pre-filter is a future optimization. |
| UTF-8 boundary issues in snippet extraction | Use `char_indices()` for offset mapping, never slice mid-char. |
| Shell quoting confusion with phrase search | Document in help text. Clap handles `"borrow checker"` as single arg naturally. |
