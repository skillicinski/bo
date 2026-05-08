# Add List Command — Implementation Plan

## Architecture decisions and rationale

- Add a dedicated `bo::list` module for list-domain behavior.
  - Keeps CLI argument parsing/output dispatch separate from tree inspection logic.
  - Makes list behavior testable without spawning the binary.
- Keep `main.rs` as a thin command layer.
  - Parse `bo list` flags with clap.
  - Require existing config using the same seeded-tree guard as other commands.
  - Call the list module and print either human-readable rows or JSON.
- Treat the markdown tree as authoritative for leaf metadata.
  - `index.jsonl` provides ordered leaf references and filename/title/url fallback data.
  - Leaf frontmatter provides `title`, `collected_at`, and `branches` when readable.
- Do not read branch files for the MVP.
  - `--branch <branch>` matches exactly against each leaf frontmatter `branches` array.
  - Missing `branches` means an empty branch array, not a degraded row.
- Prefer degraded rows over command failure for per-leaf problems.
  - Missing files, invalid frontmatter, invalid dates, invalid branch values, and suspicious paths mark the affected row as degraded.
  - Whole-command failure is reserved for unreadable config or inability to read the index/tree enough to produce a list.
- Avoid new external dependencies.
  - Existing crates already cover CLI parsing (`clap`), JSON (`serde`/`serde_json`), YAML frontmatter parsing, and RFC3339 dates (`chrono`).

## Key components and responsibilities

### `src/list.rs`

Responsibilities:

- Load leaf entries from `{tree}/index.jsonl` in existing index order.
- Resolve each index entry to a safe leaf path under the tree root.
- Read and parse leaf frontmatter when possible.
- Build `ListLeafRow` values containing display data and degradation state.
- Apply exact branch filtering.
- Apply recent ordering.
- Apply limit.
- Render human-readable output.
- Render JSON output.

Expected public API shape:

```rust
pub struct ListOptions {
    pub limit: Option<usize>,
    pub recent: bool,
    pub branch: Option<String>,
}

pub struct ListResult {
    pub leaves: Vec<ListLeafRow>,
    pub total_index_entries: usize,
    pub branch_filter: Option<String>,
}

pub fn list_leaves(tree_dir: &Path, options: &ListOptions) -> Result<ListResult, ListError>;
pub fn render_human(result: &ListResult) -> String;
pub fn render_json(result: &ListResult) -> Result<String, ListError>;
```

Exact names can change during implementation if a cleaner shape emerges.

### `src/main.rs`

Responsibilities:

- Add `List` to the `Commands` enum:
  - `--limit <n>`
  - `--recent`
  - `--branch <branch>`
  - `--json`
- Reuse `require_config()` for seeded-tree validation.
- Call `bo::list::list_leaves(...)`.
- Print JSON or human output to stdout.
- Preserve existing error behavior: failures print `error: ...` to stderr and exit non-zero.

### Existing modules

- `index`: reuse `read_index` and `IndexEntry`.
- `leaf` / `frontmatter`: reuse frontmatter parsing behavior, or parse directly through `frontmatter` where body content is not needed.
- `tree`: optional path helper use; no new storage responsibility.

## Integration points and external dependencies

- CLI integration through `clap` derive in `src/main.rs`.
- Tree location from existing config at `$HOME/.bo/config.json`.
- Leaf inventory from `{output_dir}/index.jsonl`.
- Leaf metadata from each indexed markdown file's YAML frontmatter.
- No network calls.
- No LLM/API-key dependency.
- No branch directory dependency for this MVP.
- No new crates expected.

## Implementation strategy

1. Add `pub mod list;` to `src/lib.rs`.
2. Implement list data structures and errors in `src/list.rs`.
3. Implement row construction:
   - read `index.jsonl`
   - for each entry, preserve original index position
   - reject path traversal by ensuring resolved leaf path remains under tree root
   - read frontmatter if possible
   - derive display title with fallback order:
     1. non-empty leaf frontmatter `title`
     2. non-empty index title
     3. filename stem or filename
   - derive `collected_at` from frontmatter when valid
   - derive `branches` from frontmatter sequence of strings; missing field means `[]`
   - accumulate degradation reasons instead of failing the whole list
4. Implement exact `--branch` filtering against the derived branch array.
5. Implement `--recent` sorting:
   - valid RFC3339 `collected_at` rows first, newest to oldest
   - rows without valid dates last
   - stable tie-break by original index position
6. Implement `--limit` after filtering and sorting.
7. Implement renderers:
   - human output prints clear empty/no-result messages
   - rows contain title/slug, collected date or placeholder, branch array, and `⚠ DEGRADED: ...` when degraded
   - JSON output serializes the structured result, including degradation status and reasons
8. Wire CLI command in `main.rs`.
9. Add tests.

## Testing strategy

### Unit tests in `src/list.rs`

- Empty index returns empty result.
- Default ordering follows index order.
- Missing `branches` field becomes `[]` and is not degraded.
- Branch filtering is exact.
- Missing branch filter result is empty, not an error.
- Recent ordering sorts valid dates newest-first.
- Missing/invalid dates sort after valid dates when `--recent` is used.
- Invalid frontmatter produces a degraded row with fallback title.
- Missing leaf file produces a degraded row.
- Suspicious path entries are degraded and not read outside the tree.
- Limit applies after filtering/sorting.
- JSON rendering is parseable and includes degradation fields.

### CLI integration tests

Add tests in `tests/integration_cli.rs` or a dedicated list CLI test file:

- `bo list` without seed fails with the existing seed hint.
- `bo list` on seeded empty tree reports no collected leaves.
- `bo list` prints collected leaves from a synthetic tree.
- `bo list --limit 1` prints at most one row.
- `bo list --branch <branch>` filters exactly.
- `bo list --json` emits parseable JSON.

## Validation commands

Run before implementation handoff/completion:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```
