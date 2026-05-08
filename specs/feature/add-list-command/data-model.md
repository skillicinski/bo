# Add List Command — Data Model

## Existing storage inputs

### `index.jsonl`

Existing derived cache at the tree root.

Each line deserializes as the existing `IndexEntry`:

```rust
pub struct IndexEntry {
    pub file: String,
    pub title: String,
    pub url: String,
}
```

Use in `bo list`:

- `file`: ordered pointer to the leaf markdown file.
- `title`: fallback display title when leaf frontmatter is missing or lacks a title.
- `url`: not required for human MVP output, but can be retained in internal/JSON output if useful.
- line order defines default `bo list` ordering.

Malformed index lines are already skipped by `index::read_index`; skipped malformed lines are not represented as degraded rows in this MVP.

### Leaf markdown frontmatter

Existing leaf files live at `{output_dir}/{file}` and begin with YAML frontmatter.

Relevant fields:

```yaml
title: "Example Title"
url: https://example.com/article
collected_at: 2025-06-01T10:00:00Z
updated_at: 2025-06-01T10:00:00Z
branches:
  - branch_a
  - branch_b
```

Use in `bo list`:

- `title`: preferred display title when present and non-empty.
- `collected_at`: displayed date and `--recent` sort key when parseable as RFC3339.
- `branches`: associated branch names/slugs.

Field rules:

- Missing `branches` means `[]` and is not degraded.
- `branches: []` means `[]` and is not degraded.
- A branch sequence containing non-string values is degraded; string values are still used when possible.
- Missing or empty `title` falls back to index title, then filename stem; this is not degraded by itself.
- Missing `collected_at` leaves the displayed date empty/placeholder and marks the row degraded only when date-dependent behavior requires it or the metadata is otherwise incomplete.
- Invalid `collected_at` marks the row degraded and sorts after valid dates under `--recent`.

## New in-memory entities

### `ListOptions`

Command behavior requested by the caller.

```rust
pub struct ListOptions {
    pub limit: Option<usize>,
    pub recent: bool,
    pub branch: Option<String>,
}
```

Semantics:

- `limit`: maximum number of rows after filtering and sorting.
- `recent`: sort by valid `collected_at`, newest first.
- `branch`: exact branch name/slug filter.

### `ListLeafRow`

One row emitted by `bo list`.

```rust
pub struct ListLeafRow {
    pub file: String,
    pub display_title: String,
    pub collected_at: Option<String>,
    pub branches: Vec<String>,
    pub degraded: bool,
    pub degradation_reasons: Vec<String>,
    pub index_position: usize,
}
```

Possible implementation-only fields:

```rust
pub parsed_collected_at: Option<DateTime<Utc>>
```

Field semantics:

- `file`: filename from the index entry; retained for fallback/debug/JSON use.
- `display_title`: value shown to the user, using fallback order:
  1. leaf frontmatter title
  2. index title
  3. filename stem or filename
- `collected_at`: original RFC3339 string when present and valid enough to display.
- `branches`: normalized array of branch names/slugs from leaf frontmatter; defaults to `[]`.
- `degraded`: true when the row was produced from incomplete, unreadable, unsafe, or invalid metadata.
- `degradation_reasons`: human-readable short reasons, e.g. `missing file`, `invalid frontmatter`, `invalid collected_at`, `invalid branches`, `suspicious path`.
- `index_position`: stable tie-breaker preserving default order.

### `ListResult`

Complete result before rendering.

```rust
pub struct ListResult {
    pub leaves: Vec<ListLeafRow>,
    pub total_index_entries: usize,
    pub branch_filter: Option<String>,
}
```

Semantics:

- `leaves`: rows after all filters/sorts/limits.
- `total_index_entries`: count before filters; supports empty-tree vs no-filter-results messages.
- `branch_filter`: echoed filter, if any, for clear no-result output.

### JSON output shape

Stable machine-readable output should be object-rooted, not a bare array, to allow future metadata fields.

```json
{
  "leaves": [
    {
      "file": "example.md",
      "display_title": "Example Title",
      "collected_at": "2025-06-01T10:00:00Z",
      "branches": ["branch_a", "branch_b"],
      "degraded": false,
      "degradation_reasons": []
    }
  ]
}
```

Optional metadata fields such as `total_index_entries` and `branch_filter` may be included if useful, but tests should primarily lock the per-row contract required by the spec.

## Relationships

```text
index.jsonl entry
  └── file -> leaf markdown file
          └── frontmatter branches[] -> branch names/slugs
```

`bo list` does not resolve branch names/slugs to branch markdown files in this MVP.

## Storage approach

No new persistent storage.

`bo list` is read-only and must not create, modify, delete, repair, or refresh any tree files.
