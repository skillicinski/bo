# Spec: `bo search` — Deterministic Lexical Search

## Problem statement

Users and agents have no way to find leaves in a bo tree by content. The only options are manual file browsing or `bo list` (which shows all leaves without filtering). Before `bo query` can synthesize answers, a deterministic retrieval primitive must exist that finds relevant leaves without LLM calls or network access.

## User-facing requirements

1. **Basic search**: `bo search <term> [<term>...]` returns leaves whose content (body + frontmatter) matches all provided terms.
2. **Phrase search**: a quoted argument (e.g. `"rust ownership"`) is treated as an exact phrase that must appear contiguously.
3. **Mixed**: `bo search "borrow checker" lifetime` requires the phrase "borrow checker" AND the term "lifetime" to both appear.
4. **Case insensitivity**: all matching is case-insensitive, always.
5. **Result display**: each hit shows title and a KWIC snippet (first match in body, ±80 characters of surrounding context). If the match is only in frontmatter, show the first ~160 chars of body as fallback.
6. **Result ordering**: default order is by relevance (simple term-frequency/match-density). `--recent` flag sorts by collected timestamp descending instead.
7. **Pagination**: results are returned in pages of 5. Default page is 1. `--page N` shows page N.
8. **JSON output**: `--json` flag emits structured JSON (array of `{ title, file, snippet, page, total_results }`).
9. **No mutation**: the command never modifies the tree, index, or any file.
10. **No network**: the command works entirely offline with no API keys required.
11. **Exit codes**: 0 = results found, 1 = no results, 2 = usage error.

## Success criteria

- Searching a 50+ leaf tree returns correct results in <500ms on local disk.
- AND semantics: a leaf is only returned if every term/phrase appears in its combined frontmatter + body text.
- Pagination works: `--page 2` skips the first 5 results and shows the next 5.
- `--json` output is valid JSON parseable by `jq`.
- No false negatives for exact case-insensitive substring matches.
- Command is usable without any prior `bo compile` — works on raw collected leaves.

## Out of scope

- Searching within a specific leaf or branch (narrowing scope to a subtree).
- Full BM25/tf-idf scoring — relevance ranking uses simple match density, not statistical corpus weighting.
- Highlighting/coloring matched terms in terminal output (nice-to-have, not required).
- Searching branch (compiled) content — leaves only.
- Fuzzy matching, stemming, or synonym expansion.

## Open questions

None — resolved during spec discussion.
