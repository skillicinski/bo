# Data Model: JSON output for CLI commands

## Overview

JSON mode is a presentation contract, not a new storage model. It introduces stable serializable response shapes for command results and errors. No persistent data files are added.

All responses are one JSON object written to stdout.

## Envelope

### Success envelope

```json
{
  "schema_version": 1,
  "ok": true,
  "command": "list",
  "data": {},
  "warnings": []
}
```

Fields:
- `schema_version` integer, required. Initial value: `1`.
- `ok` boolean, required. `true` for successful command outcomes, including successful no-op outcomes.
- `command` string, required. Subcommand name, or `"bo"` when no subcommand can be determined.
- `data` object, required on success. Command-specific payload.
- `warnings` array, required. Empty when there are no warnings.

### Error envelope

```json
{
  "schema_version": 1,
  "ok": false,
  "command": "show",
  "error": {
    "code": "not_found",
    "message": "leaf title 'X' not found",
    "details": {}
  },
  "warnings": []
}
```

Fields:
- `schema_version` integer, required.
- `ok` boolean, required. Always `false`.
- `command` string, required.
- `error` object, required on failure.
- `warnings` array, required.

## Error object

```json
{
  "code": "usage_error",
  "message": "missing required argument: <url>",
  "details": {}
}
```

Fields:
- `code` string, required. Stable enum-like error identifier.
- `message` string, required. Human-readable actionable explanation.
- `details` object, required. Empty when no structured context exists.

Initial error codes:
- `usage_error`
- `not_seeded`
- `duplicate_url`
- `rejected`
- `fetch_error`
- `extract_error`
- `youtube_error`
- `not_found`
- `ambiguous`
- `io_error`
- `json_error`
- `llm_error`
- `validation_error`
- `context_overflow`
- `truncated`
- `content_filter`
- `unknown_error`

## Warning object

```json
{
  "code": "degraded_leaf",
  "message": "leaf metadata is incomplete",
  "details": {
    "file": "example.md",
    "reasons": ["missing collected_at"]
  }
}
```

Fields:
- `code` string, required.
- `message` string, required.
- `details` object, required. Empty when no structured context exists.

Warnings are non-fatal. They indicate the result is usable but interpretation may require care.

## Command payloads

Payloads should stay shallow and action-oriented. Existing command result structs can be reused where they already match this model.

### `seed` data

```json
{
  "status": "created",
  "output_dir": "/absolute/path/to/tree",
  "tree_name": "research"
}
```

Fields:
- `status` string, required. Expected values: `created`, `already_seeded`.
- `output_dir` string, required.
- `tree_name` string or null, required.

Notes:
- An already-seeded tree is a successful outcome if current human behavior treats it as non-error.

### `collect` data

```json
{
  "url": "https://example.com/article",
  "file": "article.md",
  "path": "/tree/article.md"
}
```

Fields:
- `url` string, required. Normalized collected URL when available.
- `file` string, required. Leaf filename relative to the tree.
- `path` string, required if available. Full path to the written leaf.

Failure details examples:
- duplicate URL: `{ "existing_file": "article.md" }`
- rejected collection: `{ "url": "...", "reason": "..." }`

### `compile` data

Normal compile:

```json
{
  "status": "compiled",
  "reason": null,
  "branches": [
    {
      "slug": "rust-ownership",
      "title": "Rust Ownership",
      "leaf_count": 3
    }
  ],
  "leaves_updated": 5,
  "leaves_skipped": []
}
```

No-op compile:

```json
{
  "status": "noop",
  "reason": "empty_tree",
  "branches": [],
  "leaves_updated": 0,
  "leaves_skipped": []
}
```

Fields:
- `status` string, required. Expected values: `compiled`, `noop`.
- `reason` string or null, required. Expected no-op reasons: `empty_tree`, `single_leaf`, `no_valid_leaves` if represented as non-fatal.
- `branches` array, required.
- `leaves_updated` integer, required.
- `leaves_skipped` array of strings, required.

Branch fields:
- `slug` string, required.
- `title` string, required.
- `leaf_count` integer, required.

### `list` data

```json
{
  "leaves": [
    {
      "file": "article.md",
      "display_title": "Article",
      "collected_at": "2025-01-01T00:00:00Z",
      "branches": ["rust"],
      "degraded": false,
      "degradation_reasons": []
    }
  ],
  "total_index_entries": 1,
  "branch_filter": null
}
```

Fields reuse the existing list result shape:
- `leaves` array, required.
- `total_index_entries` integer, required.
- `branch_filter` string or null, required.

Leaf fields:
- `file` string, required.
- `display_title` string, required.
- `collected_at` string or null, required.
- `branches` array of strings, required.
- `degraded` boolean, required.
- `degradation_reasons` array of strings, required.

### `search` data

```json
{
  "query": {
    "terms": ["rust", "ownership"]
  },
  "hits": [
    {
      "file": "article.md",
      "title": "Article",
      "snippet": "...ownership...",
      "collected_at": "2025-01-01T00:00:00Z"
    }
  ],
  "total_results": 1,
  "page": 1,
  "total_pages": 1
}
```

Fields:
- `query` object, required.
- `query.terms` array of strings, required. Terms after CLI parsing/lowercasing if that is the command's matching behavior.
- `hits` array, required. Empty array is success.
- `total_results` integer, required.
- `page` integer, required.
- `total_pages` integer, required.

Hit fields:
- `file` string, required.
- `title` string, required.
- `snippet` string, required.
- `collected_at` string or null, required.

Internal relevance score should remain omitted unless needed for user actionability.

### `show` data

```json
{
  "leaf": {
    "title": "Article",
    "file": "article.md",
    "path": "/tree/article.md",
    "url": "https://example.com/article",
    "frontmatter": {},
    "frontmatter_raw": "---\n...\n---\n",
    "body": "# Article\n...",
    "truncated": false,
    "full": true
  }
}
```

Fields reuse the existing show result shape:
- `leaf` object, required.
- `leaf.title` string, required.
- `leaf.file` string, required.
- `leaf.path` string, required.
- `leaf.url` string or null, required.
- `leaf.frontmatter` object, required.
- `leaf.frontmatter_raw` string, required.
- `leaf.body` string, required.
- `leaf.truncated` boolean, required.
- `leaf.full` boolean, required.

Ambiguous error details:

```json
{
  "title": "Article",
  "candidates": [
    {
      "file": "a.md",
      "title": "Article",
      "path": "/tree/a.md",
      "url": "https://example.com/a"
    }
  ]
}
```

### `raze` data

```json
{
  "deleted_files": 3,
  "deleted_index": true,
  "removed_output_dir": false,
  "output_dir_left_in_place": true,
  "deleted_config": true,
  "output_dir": "/tree",
  "config_path": "/home/user/.bo/config.json"
}
```

Fields:
- `deleted_files` integer, required.
- `deleted_index` boolean, required.
- `removed_output_dir` boolean, required.
- `output_dir_left_in_place` boolean, required.
- `deleted_config` boolean, required.
- `output_dir` string, required.
- `config_path` string, required.

Warnings may include skipped suspicious ledger entries.

## Relationships

- Every command payload is nested under one envelope.
- Warnings are top-level even when they originate from command-specific items.
- Per-item degradation remains on list rows; top-level warnings can summarize or duplicate high-value degraded states.
- Errors replace `data`; successful no-op states remain `data` with `ok: true`.

## Storage approach

No persistent storage changes.

The feature only changes CLI output serialization and command result plumbing. Existing tree files, markdown leaves, branch files, config files, and `index.jsonl` remain unchanged.
