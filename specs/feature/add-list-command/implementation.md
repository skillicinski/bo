# Add List Command — Implementation Summary

## Overview

Implemented a read-only `bo list` command for deterministic inspection of collected leaves in the active tree.

The command lists leaves from `index.jsonl`, enriches rows with leaf frontmatter metadata, preserves index order by default, and supports recent sorting, exact branch filtering, row limiting, and JSON output.

## User-facing behavior

Supported command shape:

```bash
bo list
bo list --limit <n>
bo list --recent
bo list --branch <branch>
bo list --json
```

Flags can be combined, e.g.:

```bash
bo list --branch branch_a --recent --limit 5 --json
```

Human output shows:

```text
<title-or-fallback> | <collected_at-or-> | [branch_a, branch_b]
```

Special cases:

- Empty tree: `no leaves collected yet`
- Branch filter with no matches: `no leaves matched branch '<branch>'`
- Degraded row: appends a non-color-only marker, e.g. `⚠ DEGRADED: missing file`

JSON output is object-rooted and includes per-row degradation metadata:

```json
{
  "leaves": [
    {
      "file": "example.md",
      "display_title": "Example Title",
      "collected_at": "2025-06-01T10:00:00Z",
      "branches": ["branch_a"],
      "degraded": false,
      "degradation_reasons": []
    }
  ]
}
```

## Implementation details

### New module: `src/list.rs`

Added the list domain module with:

- `ListOptions`
- `ListLeafRow`
- `ListResult`
- `ListError`
- `list_leaves(...)`
- `render_human(...)`
- `render_json(...)`

Core behavior:

- Reads `{tree}/index.jsonl` through existing `index::read_index`.
- Preserves index order via `index_position`.
- Reads leaf frontmatter through existing `frontmatter::parse`.
- Display title fallback order:
  1. non-empty leaf frontmatter `title`
  2. non-empty index title
  3. filename stem / filename
- Missing `branches` means `[]` and is not degraded.
- Invalid `branches` marks the row degraded while preserving valid string branch values.
- Missing or invalid `collected_at` marks the row degraded.
- `--recent` sorts valid RFC3339 dates newest-first, then invalid/missing dates, preserving index order for ties.
- `--branch` performs exact matching against derived branch strings.
- `--limit` applies after filtering and sorting.
- Suspicious paths and path traversal attempts are degraded and never read outside the tree.
- Missing/unreadable/invalid leaf files produce degraded rows instead of failing the whole command.

### CLI wiring: `src/main.rs`

Added `List` to the clap subcommands with flags:

- `--limit <LIMIT>`
- `--recent`
- `--branch <BRANCH>`
- `--json`

The command reuses existing config/seed validation and existing CLI error conventions.

### Library export: `src/lib.rs`

Exported the new module:

```rust
pub mod list;
```

### Documentation: `README.md`

Added `bo list` to the command reference:

```text
bo list [--recent] [--branch <branch>] [--limit <n>] [--json]  # List collected leaves
```

## Tests added

### Unit tests in `src/list.rs`

Covered:

- empty index
- default index ordering
- suspicious path degradation
- missing file degradation
- invalid frontmatter degradation
- display title fallback behavior
- valid/missing/invalid `collected_at`
- missing/empty/string/mixed/scalar `branches`
- exact branch filtering
- no-match branch filtering
- recent sorting
- limit after filtering/sorting
- read-only behavior
- human renderer output
- empty/no-result renderer messages
- degraded renderer marker
- JSON renderer parseability and omitted internal `index_position`

### CLI integration tests in `tests/integration_cli.rs`

Covered:

- `bo list` without seed fails with seed hint
- empty seeded tree reports no collected leaves
- synthetic tree lists leaves in index order with dates and branch arrays
- `--limit 1`
- exact `--branch` filtering and no-match success
- `--json` parseability and degradation fields
- combined `--branch --recent --limit --json`

## Validation

Final validation passed:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Notes

- No new dependencies were added.
- No network or LLM/API-key path is involved.
- The command is read-only and does not create, modify, repair, refresh, or delete tree files.
- Branch files are not read; branch filtering uses leaf frontmatter branch arrays only.
