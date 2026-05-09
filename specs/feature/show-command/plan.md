# Add Show Command — Implementation Plan

## Architecture decisions and rationale

- Add a dedicated `bo::cli::show` module for show-domain behavior.
  - Keeps CLI argument parsing/output dispatch in `main.rs` thin.
  - Keeps leaf lookup, parsing, previewing, and rendering testable without spawning the binary.
- Match the existing `bo list` architecture.
  - `bo list` already provides the nearest read-only inspection pattern.
  - Reuse its conventions for index loading, safe path resolution, JSON rendering, and error style where appropriate.
- Treat the markdown tree as authoritative.
  - `index.jsonl` is used to discover candidate leaf files and provide fallback identity data.
  - Each leaf markdown file provides the stored frontmatter and body content.
- Limit MVP lookup to case-insensitive exact leaf title.
  - Prefer leaf frontmatter `title` when readable.
  - Use the index title only as fallback when the leaf cannot provide a usable title.
  - Do not perform fuzzy, partial, prefix, slug, path, or URL lookup.
- Fail clearly rather than guessing.
  - No match is an error with a `bo list` suggestion.
  - Duplicate matching titles are an ambiguity error with candidate details.
  - Missing/unreadable/invalid selected leaves are errors because `bo show` is asked to display one specific leaf, not degrade rows like `bo list`.
- Keep preview deterministic and bounded.
  - Default output includes stored frontmatter and a bounded body preview.
  - `--full` returns the full body.
  - Exact preview size can be an internal constant; tests should assert truncation behavior without over-specifying terminal layout.
- Avoid new dependencies.
  - Existing crates already cover CLI parsing, JSON serialization, frontmatter parsing, and filesystem access.

## Key components and responsibilities

### `src/cli/show.rs`

Responsibilities:

- Load candidate leaves from `{tree}/index.jsonl` in existing index order.
- Resolve indexed leaf paths safely under the tree root.
- Read and parse leaf markdown content into:
  - raw/stored frontmatter text for human display
  - parsed frontmatter mapping for title matching and JSON output
  - body content for preview/full output
- Match leaf title case-insensitively and exactly.
- Detect no-match and duplicate-title cases.
- Build a structured `ShowResult` for the selected leaf.
- Render human-readable output.
- Render JSON output.

Expected public API shape:

```rust
pub struct ShowOptions {
    pub full: bool,
}

pub struct ShowResult {
    pub title: String,
    pub file: String,
    pub path: String,
    pub frontmatter: serde_yaml_ng::Mapping,
    pub frontmatter_raw: String,
    pub body: String,
    pub truncated: bool,
}

pub fn show_leaf(tree_dir: &Path, title: &str, options: &ShowOptions) -> Result<ShowResult, ShowError>;
pub fn render_human(result: &ShowResult) -> String;
pub fn render_json(result: &ShowResult) -> Result<String, ShowError>;
```

Exact type and field names may change during implementation if a cleaner shape emerges.

### `src/cli/mod.rs`

Responsibilities:

- Export the new `show` module with `pub mod show;`.

### `src/main.rs`

Responsibilities:

- Add `Show` to the clap `Commands` enum:
  - positional `<title>`
  - `--full`
  - `--json`
- Reuse `require_config()` for seeded-tree validation.
- Call `bo::cli::show::show_leaf(...)`.
- Print JSON or human output to stdout.
- Preserve existing error convention: failures print `error: ...` to stderr and exit non-zero.

### Existing modules

- `domain::index`: reuse `read_index` and `IndexEntry` for leaf inventory.
- `domain::frontmatter`: reuse parse behavior where useful, but add local raw-frontmatter splitting in `show` if needed to display stored frontmatter as-is.
- `cli::list`: optionally mirror safe path resolution and fallback title behavior. If shared code becomes necessary, extract it only if it reduces duplication without broadening scope.

## Integration points and external dependencies

- CLI integration through `clap` derive in `src/main.rs`.
- Tree location from existing config at `$HOME/.bo/config.json`.
- Leaf inventory from `{output_dir}/index.jsonl`.
- Leaf metadata and body from indexed markdown files.
- No network calls.
- No LLM/API-key dependency.
- No branch file dependency for this MVP.
- No new crates expected.

## Implementation strategy

1. Add `src/cli/show.rs` and export it from `src/cli/mod.rs`.
2. Define show data structures and errors:
   - options (`full`)
   - selected leaf result
   - display body variant or fields (`body`, `truncated`)
   - not found, ambiguous, unreadable, invalid-frontmatter, suspicious-path, I/O, and JSON errors
3. Implement safe path resolution for indexed leaf files:
   - reject absolute paths, empty paths, parent directory traversal, and Windows prefix paths where relevant
   - ensure existing candidate files resolve under the tree root when canonicalization is possible
4. Implement leaf loading:
   - read candidate file
   - split markdown into raw frontmatter and body
   - parse frontmatter mapping
   - extract usable title from frontmatter, falling back to index title if needed
5. Implement exact case-insensitive title matching:
   - normalize with Unicode-aware lowercasing or equivalent standard string comparison
   - compare full normalized strings only
   - collect all matching candidates before selecting
6. Implement selection errors:
   - zero matches: not-found error that suggests `bo list`
   - multiple matches: ambiguous-title error with candidate file/path/title details
   - selected leaf missing/unreadable/invalid: fail with a clear reason
7. Implement preview behavior:
   - default mode returns a bounded body preview and `truncated = true` when content is omitted
   - `--full` returns complete body and `truncated = false`
   - preserve body text as stored rather than performing rich markdown rendering
8. Implement renderers:
   - human output prints raw frontmatter as stored, then preview/full body
   - preview output visibly indicates truncation
   - JSON output is object-rooted and includes selected leaf identity, parsed frontmatter, body or preview, and truncation state
9. Wire CLI command in `main.rs`.
10. Add tests.

## Testing strategy

### Unit tests in `src/cli/show.rs`

- Empty index produces not-found with `bo list` suggestion.
- Case-insensitive exact match finds the intended leaf.
- Partial title does not match.
- Duplicate matching titles produce an ambiguous-title error with candidate details.
- Missing selected file produces a clear display error.
- Invalid selected frontmatter produces a clear display error.
- Suspicious/path-traversal index entries are rejected and not read outside the tree.
- Default mode returns bounded preview and marks truncated output when applicable.
- Default mode does not mark short bodies as truncated.
- `full` mode returns complete body and is not truncated.
- Human renderer includes raw frontmatter and body preview/full content.
- Human renderer visibly indicates truncation.
- JSON renderer emits parseable object-rooted JSON with leaf identity, frontmatter, body/preview, and truncation state.
- Read-only behavior: showing a leaf does not create, modify, or delete tree files.

### CLI integration tests

Add tests in `tests/integration_cli.rs`:

- `bo show <title>` without seed fails with the existing seed hint.
- `bo show "Some Title"` on a seeded synthetic tree prints frontmatter and a preview.
- title matching is case-insensitive.
- partial title lookup fails.
- `bo show --full "Some Title"` prints full body.
- `bo show --json "Some Title"` emits parseable JSON with required fields and truncated state.
- `bo show --json --full "Some Title"` emits parseable JSON with full body and `truncated = false`.
- missing title exits non-zero and suggests `bo list`.
- duplicated title exits non-zero and reports ambiguity.

## Validation commands

Run before implementation handoff/completion:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
