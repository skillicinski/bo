# Tasks: `bo search` Implementation

## Scaffold

- [x] Create `src/cli/search.rs` with public types (`SearchQuery`, `SearchOptions`, `SearchResult`, `SearchHit`), constants (`PAGE_SIZE`, `SNIPPET_RADIUS`, `FALLBACK_SNIPPET_LEN`), and a `search_leaves()` stub that returns `todo!()`.
- [x] Register module in `src/cli/mod.rs` (`pub mod search;`).
- [x] Add `Search` variant to `Commands` enum in `src/main.rs` with clap args: positional `terms: Vec<String>`, `--page`, `--recent`, `--json`. Add stub `cmd_search` handler. Confirm project compiles.

## Core matching

- [x] Implement `matches_all_terms(content_lower: &str, terms_lower: &[String]) -> bool` — returns true only if every term is a substring of content.
- [x] Implement `score_relevance(content_lower: &str, terms_lower: &[String]) -> usize` — `(sum of content.matches(term).count() * 1000) / content.len()`. Per-mille density normalization.
- [x] Unit tests: single term, multiple terms AND, phrase as single arg, case insensitivity, no-match returns false/0, overlapping matches counted correctly.

## Snippet extraction

- [x] Implement `extract_snippet(body: &str, terms_lower: &[String], radius: usize) -> String` — finds first occurrence of any term in lowercased body, returns ±radius chars (UTF-8 safe via `char_indices()`), prepends/appends `…` when truncated. Collapse `\n+` to single space in returned snippet.
- [x] Implement fallback path: if no term found in body, return first `FALLBACK_SNIPPET_LEN` chars of body with trailing `…`.
- [x] Unit tests: match at start, match at end, match in middle, multi-byte UTF-8 boundaries, fallback when match is frontmatter-only, empty body, body shorter than radius, newlines collapsed to space.

## Orchestration, sorting, and pagination

- [x] Implement `search_leaves(tree_dir: &Path, query: &SearchQuery, options: &SearchOptions) -> Result<SearchResult, SearchError>` — read index, load each leaf (skip missing/unreadable files silently), filter by `matches_all_terms`, score, sort, paginate.
- [x] Default sort: score descending, tie-break by index position ascending.
- [x] `--recent` sort: `collected_at` descending (reuse date-parsing pattern from `list.rs`), undated leaves sink to bottom, tie-break by index position.
- [x] Pagination: skip `(page - 1) * PAGE_SIZE`, take `PAGE_SIZE`. Compute `total_pages`.
- [x] TempDir-based tests: multi-leaf tree with known content, verify correct hits returned, AND semantics enforced, ordering stable, pagination slices correctly, out-of-range page returns empty hits with correct total.

## Rendering

- [x] Implement `render_human(result: &SearchResult) -> String` — each hit as `title\n  snippet\n\n`, plus footer with page/total info. Empty results message.
- [x] Implement `render_json(result: &SearchResult) -> Result<String, SearchError>` — pretty-printed JSON with `hits`, `total_results`, `page`, `total_pages`.
- [x] Unit tests: formatting correctness, JSON is valid and parseable, empty result rendering.

## CLI integration

- [x] Replace `cmd_search` stub in `src/main.rs`: call `search::search_leaves`, select renderer by `--json` flag, print output, exit 0 on results, exit 1 on no results.
- [x] Validate `--page` is ≥1 at parse time (clap `value_parser` or manual check with exit 2).
- [x] Validate at least one term is provided (clap `required = true` on positional or manual check).

## Integration test

- [x] Add `tests/integration_search.rs`: seed a tree with 10+ leaves of varied content, test basic single-term search, multi-term AND, phrase search, `--recent` ordering, `--page 2`, `--json` output parseable by serde_json, exit code 1 on no match.
