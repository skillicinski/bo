# Implementation: `bo search`

## Summary

Deterministic lexical search over collected leaves. Single session, branch `feature/search-command`.

## What shipped

### New files
- `src/cli/search.rs` — core module (~490 LOC implementation, ~420 LOC tests)
- `tests/integration_search.rs` — 10 end-to-end tests against real binary

### Modified files
- `src/cli/mod.rs` — registered `pub mod search`
- `src/main.rs` — added `Search` command variant, `cmd_search` handler, exit code logic

### No new dependencies
Pure stdlib string operations. No new crates added to `Cargo.toml`.

## Architecture

Follows the `list.rs` pattern exactly: a pure library function (`search_leaves`) takes tree dir + options, returns a typed result struct. CLI layer in `main.rs` handles rendering and exit codes.

```
main.rs:cmd_search
  → search::search_leaves(tree_dir, query, options)
    → index::read_index (enumerate leaf paths)
    → fs::read_to_string (load each leaf)
    → matches_all_terms (AND gate)
    → score_relevance (word-count density)
    → sort + paginate
    → extract_snippet (KWIC window)
  → render_human / render_json
  → exit code
```

## Key decisions made during implementation

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Scoring normalization | Word count, not char count | Char count produced scores of 0–2 on 50KB docs due to integer division. Word count gives meaningful differentiation. |
| Score visibility | `#[serde(skip)]` — hidden from JSON | Internal ranking detail, not a stable API. Avoids contract breakage if scoring changes. |
| Match corpus | Entire file content (grep behavior) | Simplest, no false negatives. Searching "title" matches all leaves — acceptable tradeoff. |
| Snippet newlines | Collapsed to single space | Cleaner terminal output. Uses `is_ascii_whitespace()` to collapse all runs. |
| Page out-of-range | Exit 1 (no results on screen) | Simpler mental model. `total_results` in JSON still communicates matches exist. |
| Missing/unreadable files | Skipped silently | Follows `list.rs` graceful degradation pattern. |
| `--page` validation | Manual check in `cmd_search`, exit 2 | Clap's `value_parser!().range()` not available without extra features. |

## Test coverage

- **36 unit tests** in `src/cli/search.rs`:
  - Core matching: single term, multi-term AND, phrase, pre-lowercase contract
  - Scoring: empty content, single/multiple occurrences, density comparison, overlapping matches
  - Snippets: start/middle/end positions, UTF-8 safety, fallback, truncation, newline collapse
  - Orchestration: basic search, AND semantics, phrase, case insensitivity, relevance ordering, recent ordering, pagination (page 1, page 2, out-of-range), missing files, frontmatter matches
  - Rendering: human format, empty results, empty page, JSON validity, score hidden

- **10 integration tests** in `tests/integration_search.rs`:
  - 11-leaf corpus with varied content (Rust, Python, Go)
  - Single term, multi-term AND, phrase matching
  - Case insensitivity, `--recent` flag, `--page 2`
  - JSON output parseable by serde_json
  - Exit code 1 on no match and out-of-range page

## Dogfood results

Tested against existing `tmp-tree` (5 Rust Book chapters, 2 compiled branches):

- Single term (`ownership`) → 4 hits, correctly ranked by density
- Multi-term AND (`rust trait generic`) → 1 hit (traits chapter)
- Phrase (`"smart pointer"`) → 1 hit (Rc chapter)
- `--recent` reorders by collected date
- `--json` pipes cleanly to `jq`
- Exit 1 on no match
- Branches correctly excluded (only index entries = leaves searched)

Word-count normalization produces meaningful score differentiation even on large (~50KB) documents.

## Deviations from plan

- `parse_query` was not implemented as a separate function — trivial one-liner (lowercase the clap args) done inline in `cmd_search`.
- `ScoredLeaf` kept as a private struct rather than a separate type in a domain module — no need for reuse outside search.
- Snippet extraction works on byte positions mapped through `char_indices()` rather than converting to char offsets first — same safety guarantee, slightly more direct.
