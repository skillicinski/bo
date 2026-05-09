# Add Show Command — Implementation Summary

## Overview

Implemented a read-only `bo show` command for deterministic inspection of a single collected leaf by exact case-insensitive title.

The command loads candidates from `index.jsonl`, reads the selected leaf markdown file, preserves stored frontmatter for human output, emits a bounded body preview by default, and supports full-body and JSON output modes.

## User-facing behavior

Supported command shape:

```bash
bo show <title>
bo show --full <title>
bo show --json <title>
bo show --json --full <title>
```

Default human output prints:

- raw stored frontmatter, including delimiters
- bounded body preview
- a visible truncation marker when content is omitted

`--full` prints the full body and no truncation marker.

JSON output is object-rooted:

```json
{
  "leaf": {
    "title": "Example Title",
    "file": "example-title.md",
    "path": "/abs/tree/example-title.md",
    "url": "https://example.com/article",
    "frontmatter": { "title": "Example Title" },
    "frontmatter_raw": "---\ntitle: \"Example Title\"\n---\n",
    "body": "# Example Title\n\nPreview or full body",
    "truncated": false,
    "full": false
  }
}
```

Error behavior:

- no matching title: exits non-zero, reports not found, suggests `bo list`
- duplicate matching titles: exits non-zero, reports ambiguity with candidate details
- selected missing/unreadable/invalid leaf: exits non-zero with a clear reason
- suspicious index paths are rejected and never read outside the tree

## Implementation details

### New module: `src/cli/show.rs`

Added:

- `ShowOptions`
- `ShowCandidateSummary`
- `ShowResult`
- `ShowError`
- `show_leaf(...)`
- `render_human(...)`
- `render_json(...)`

Core behavior:

- Reads `{tree}/index.jsonl` through existing `domain::index::read_index`.
- Resolves indexed leaf paths safely under the tree root.
- Reads leaf markdown files and splits them into raw frontmatter, parsed frontmatter, and body.
- Matches by exact case-insensitive title.
- Uses non-empty frontmatter `title` first, then non-empty index title fallback.
- Detects ambiguity before selecting a leaf.
- Uses a deterministic 2,000-character default preview.
- Includes both parsed `frontmatter` and `frontmatter_raw` in JSON for agent workflows.

### CLI wiring: `src/main.rs`

Added `Show` to the clap subcommands with:

- positional `<title>`
- `--full`
- `--json`

The command reuses existing config/seed validation and CLI error conventions.

### Library export: `src/cli/mod.rs`

Exported the new module:

```rust
pub mod show;
```

### Documentation: `README.md`

Added `bo show` to the command reference.

## Tests added

### Unit tests in `src/cli/show.rs`

Covered:

- empty index not-found behavior
- suspicious path rejection
- raw frontmatter preservation
- title fallback behavior
- case-insensitive exact matching
- partial title not matching
- not-found message with `bo list` suggestion
- duplicate-title ambiguity
- missing/unreadable/invalid selected leaves
- bounded preview and full body behavior
- read-only behavior
- human rendering
- JSON rendering

### CLI integration tests in `tests/integration_cli.rs`

Covered:

- `bo show` without seed fails with existing seed hint
- default show prints frontmatter and bounded preview
- case-insensitive exact title matching
- `--full` prints complete body
- `--json` emits parseable structured output
- `--json --full` emits complete body with `truncated = false`
- missing title reports not found and suggests `bo list`
- duplicate title reports ambiguity with candidate details

## Validation

Final validation passed:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Dogfood

Ran a default-corpus subset into an isolated temp tree:

```text
/var/folders/_j/hthk3s914yx97zyftktwj71h0000gn/T/bo-show-dogfood-default.Hh25CC
```

Collected 4/5 URLs successfully:

- Rust Book ownership chapter — collected, but title extraction produced `Keyboard shortcuts`
- Rust Book concurrency chapter — collected, but title extraction produced `Keyboard shortcuts`
- Rust language Wikipedia page — collected
- React Quick Start — collected
- Rust blog traits URL — rejected as `redirect stub`

Show command checks:

- `bo show "Rust (programming language) - Wikipedia"` printed raw frontmatter and a truncated preview.
- `bo show --json "Quick Start – React"` emitted parseable JSON with `truncated = true`, `full = false`, and a 2,000-character body preview.
- `bo show "Keyboard shortcuts"` failed as ambiguous and listed both Rust Book candidate files/URLs.
- `bo show --full "Quick Start – React"` printed the complete body.

Compile was attempted against the dogfood tree but was blocked because `OPENAI_API_KEY` was not set in this session:

```text
error: OPENAI_API_KEY is not set — bo compile requires an OpenAI API key
```

Post-compile-attempt `bo show --json` still worked and showed no branch frontmatter updates, as expected because compile did not run.

## Notes

- No new dependencies were added.
- No network or LLM/API-key path is involved in `bo show`.
- The command is read-only and does not create, modify, repair, refresh, or delete tree files.
- Lookup intentionally remains leaf-only and exact case-insensitive title-only for this scope.
- Dogfood confirmed existing title-extraction pollution makes two Rust Book pages ambiguous by title; slug/path lookup remains out of scope for this feature.
