# Add Show Command

## Problem statement

Users and agents currently have no simple deterministic way to inspect the contents of a single collected leaf from a `bo` tree. After listing leaves, they must open markdown files manually to see a leaf's stored metadata and content.

This makes progressive inspection awkward: users cannot quickly move from "what leaves exist?" to "what does this leaf contain?" without knowing the tree's file layout.

The feature should add a read-only `bo show` command for displaying one collected leaf by title.

## User-facing requirements

- A user can run `bo show <title>` to display one collected leaf whose title exactly matches `<title>` case-insensitively.
- Titles containing spaces are accepted when passed as a normal quoted shell argument, e.g. `bo show "Understanding Ownership"`.
- Matching is exact aside from case. Partial, fuzzy, prefix, URL, path, and slug lookup are not part of this first scope.
- `bo show` is read-only: it does not collect, compile, repair, rewrite, refresh, or otherwise mutate the tree.
- By default, `bo show <title>` prints:
  - the selected leaf's stored frontmatter as-is
  - a bounded preview of the leaf body
  - a visible indication when the body has been truncated for preview
- `bo show --full <title>` prints the selected leaf's stored frontmatter as-is and the full leaf body.
- `bo show --json <title>` emits a machine-readable representation suitable for agent workflows and progressive disclosure.
- JSON output includes enough stable information for an agent to identify the selected leaf and decide whether to request the full body, including:
  - the matched title
  - the leaf slug or file identifier when available
  - the leaf file path when available
  - the stored frontmatter
  - the preview body for default mode
  - whether the body is truncated
  - the full body when `--full` is used
- `--json` and `--full` can be combined.
- If no leaf title matches, `bo show` exits unsuccessfully, reports that the leaf was not found, and suggests running `bo list` to inspect available leaves.
- If multiple leaves have the same matching title, `bo show` exits unsuccessfully, reports the title as ambiguous, and shows enough candidate information to explain the ambiguity.
- If the tree cannot be read enough to identify leaves, `bo show` exits unsuccessfully with a clear tree-read error.
- If the selected leaf cannot be displayed because its stored content is missing or unreadable, `bo show` exits unsuccessfully with a clear reason.
- Human-readable errors should be concise and actionable. JSON errors are not required for this first scope.
- The command does not require network access or an LLM/API key.

## Success criteria

- Running `bo show "Some Title"` on a tree with one leaf titled `Some Title` prints that leaf's frontmatter and a bounded body preview.
- Title matching is case-insensitive: `bo show "some title"` finds a leaf titled `Some Title`.
- Title matching is exact: `bo show "Some"` does not match `Some Title` solely as a partial title.
- Running `bo show --full "Some Title"` prints the same frontmatter and the full body.
- Default preview output visibly indicates when additional body content is omitted.
- Running `bo show --json "Some Title"` emits parseable JSON containing the selected leaf identity, frontmatter, preview content, and truncation state.
- Running `bo show --json --full "Some Title"` emits parseable JSON containing the selected leaf identity, frontmatter, and full body content.
- Running `bo show "Missing Title"` exits unsuccessfully with a not-found message and suggests `bo list`.
- Running `bo show` for a duplicated title exits unsuccessfully with an ambiguous-title message and candidate details.
- Running `bo show` does not create, modify, or delete tree files.
- The command works without network access or an LLM/API key.

## Out of scope

- Showing compiled branches.
- Lookup by slug, file path, URL, unique prefix, partial title, or fuzzy title match.
- Full-text search, lexical ranking, snippets from arbitrary matches, or `bo search` behaviour.
- Listing leaves or branches beyond candidate details needed for ambiguity errors.
- Tree health diagnostics beyond errors needed to explain why the selected leaf cannot be shown.
- Repairing, pruning, rebuilding indices, or modifying metadata.
- Network fetching, source revalidation, or refreshing collection metadata.
- Rich terminal rendering, paging, TUI behaviour, syntax highlighting, or exact table/layout styling.
- JSON output for error cases.

## Open questions

None for first scope.
