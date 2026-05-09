# Add Show Command — Data Model

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

Use in `bo show`:

- `file`: pointer to the leaf markdown file.
- `title`: fallback title when the leaf frontmatter is missing a usable title.
- `url`: fallback/source identity useful for candidate details and JSON output.
- line order provides deterministic candidate ordering for ambiguity details.

Malformed index lines are already skipped by `index::read_index`; skipped malformed lines are not candidates for `bo show` in this MVP.

### Leaf markdown file

Existing leaf files live at `{output_dir}/{file}` and begin with YAML frontmatter followed by a markdown body.

Representative shape:

```yaml
---
title: "Example Title"
url: https://example.com/article
collected_at: 2025-06-01T10:00:00Z
updated_at: 2025-06-01T10:00:00Z
branches:
  - branch_a
---

# Example Title

Body content.
```

Use in `bo show`:

- raw frontmatter text: displayed as-is in human output.
- parsed frontmatter mapping: used for title matching and JSON output.
- `title`: primary lookup field.
- body: previewed by default, emitted fully with `--full`.

Field rules:

- A non-empty frontmatter `title` is the preferred match/display title.
- If frontmatter lacks a usable title, the index title may be used as fallback identity.
- Matching remains exact and case-insensitive against the resulting title.
- Missing, unreadable, unsafe, or invalid candidate files are not silently selected; if they prevent showing the requested leaf, the command fails with a clear reason.

## New in-memory entities

### `ShowOptions`

Command behavior requested by the caller.

```rust
pub struct ShowOptions {
    pub full: bool,
}
```

Semantics:

- `full = false`: include bounded body preview.
- `full = true`: include complete body.

### `ShowCandidate`

Internal candidate built from an index entry.

```rust
pub struct ShowCandidate {
    pub file: String,
    pub title: String,
    pub url: Option<String>,
    pub path: PathBuf,
    pub index_position: usize,
}
```

Semantics:

- `file`: index file value.
- `title`: title used for exact case-insensitive matching.
- `url`: source identity, if available.
- `path`: resolved safe path under the tree root.
- `index_position`: deterministic ordering for ambiguity output.

This type may remain private.

### `ShowResult`

Selected leaf data before rendering.

```rust
pub struct ShowResult {
    pub title: String,
    pub file: String,
    pub path: String,
    pub url: Option<String>,
    pub frontmatter: serde_yaml_ng::Mapping,
    pub frontmatter_raw: String,
    pub body: String,
    pub truncated: bool,
    pub full: bool,
}
```

Semantics:

- `title`: matched title.
- `file`: index file identifier.
- `path`: resolved leaf path for agent navigation/debugging.
- `url`: source URL when known.
- `frontmatter`: parsed YAML object for JSON output.
- `frontmatter_raw`: stored frontmatter text for human output.
- `body`: preview body in default mode, full body in `--full` mode.
- `truncated`: true only when default preview omitted body content.
- `full`: echoes whether the caller requested full content.

Exact field names can change, but JSON output should preserve these concepts.

### `ShowError`

Command/domain errors.

Expected variants/concepts:

```rust
pub enum ShowError {
    Io(std::io::Error),
    Json(serde_json::Error),
    NotFound { title: String },
    Ambiguous { title: String, candidates: Vec<ShowCandidateSummary> },
    SuspiciousPath { file: String },
    MissingFile { file: String },
    UnreadableFile { file: String },
    InvalidFrontmatter { file: String },
}
```

Human messages should be concise and actionable:

- not found: mention the title and suggest `bo list`.
- ambiguous: mention the title and show candidate files/paths.
- unreadable/invalid selected leaf: mention the file and reason.

### `ShowJsonOutput`

Stable machine-readable output should be object-rooted, not a bare value, to allow future metadata fields.

Default preview mode:

```json
{
  "leaf": {
    "title": "Example Title",
    "file": "example-title.md",
    "path": "/abs/tree/example-title.md",
    "url": "https://example.com/article",
    "frontmatter": {
      "title": "Example Title",
      "url": "https://example.com/article",
      "collected_at": "2025-06-01T10:00:00Z"
    },
    "body": "# Example Title\n\nPreview...",
    "truncated": true,
    "full": false
  }
}
```

Full mode:

```json
{
  "leaf": {
    "title": "Example Title",
    "file": "example-title.md",
    "path": "/abs/tree/example-title.md",
    "url": "https://example.com/article",
    "frontmatter": { "title": "Example Title" },
    "body": "# Example Title\n\nComplete body...",
    "truncated": false,
    "full": true
  }
}
```

JSON output for error cases is out of scope for this first feature.

## Relationships

```text
index.jsonl entry
  └── file -> leaf markdown file
          ├── frontmatter.title -> exact case-insensitive lookup
          ├── frontmatter -> displayed/serialized metadata
          └── body -> preview or full content
```

`bo show` does not resolve branch names/slugs to branch markdown files in this MVP.

## Storage approach

No new persistent storage.

`bo show` is read-only and must not create, modify, delete, repair, refresh, or rewrite any tree files.
