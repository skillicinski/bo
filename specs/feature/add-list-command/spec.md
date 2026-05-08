# Add List Command

## Problem statement

Users and agents currently have no simple deterministic way to inspect what has been collected into a `bo` tree. After collecting and compiling documents, they must browse markdown files manually to see which leaves exist, when they were collected, and which branches they are linked to.

This makes the tree harder to trust, navigate, and use as the basis for later search/query workflows.

The feature should add a read-only `bo list` command that lists collected leaves in a concise, predictable form.

## User-facing requirements

- A user can run `bo list` to list collected leaves in the current tree.
- `bo list` is read-only: it does not collect, compile, repair, rewrite, or otherwise mutate the tree.
- By default, `bo list` lists leaves only.
- Default output order follows the tree/index order.
- Each listed leaf shows:
  - a title when available, otherwise a slug/filename fallback
  - the collected date when available
  - the associated branch names/slugs as an array, e.g. `[branch_a, branch_b]`
- Leaves with no associated branches show an empty branch array, e.g. `[]`.
- If there are no leaves, `bo list` prints a clear empty-tree message.
- `bo list --limit <n>` shows at most `n` leaves.
- `bo list --recent` lists leaves by collected date, newest first.
- In recent ordering, leaves without a usable collected date still appear, but after leaves with usable dates and marked as degraded if appropriate.
- `bo list --branch <branch>` lists only leaves associated with the selected branch name/slug.
- If a branch filter matches no leaves, `bo list` prints a clear no-results message.
- `bo list --json` emits a machine-readable representation of the same list data.
- JSON output includes, for each leaf, the displayed title/slug value, collected date when available, branch array, and degradation status.
- `--limit`, `--recent`, `--branch`, and `--json` can be combined.
- If an individual leaf has missing, incomplete, or invalid metadata, `bo list` should still show the row when possible, using fallback values and marking it as degraded.
- Degraded rows must be visibly marked in human-readable output with a non-color-only indicator or label. Color may be used as an enhancement but must not be required to understand the status.
- A degraded row should not cause the whole command to fail unless the tree itself cannot be read enough to produce a meaningful list.

## Success criteria

- Running `bo list` on a tree with collected leaves prints one row per leaf in tree/index order.
- Each normal row includes a display title or slug, a collected date when available, and a branch array.
- A leaf with branch links displays those branches in its row.
- A leaf without branch links displays `[]` for branches.
- `bo list --limit 1` prints at most one leaf.
- `bo list --recent` orders dated leaves newest-first.
- `bo list --branch <branch>` prints only leaves associated with that branch.
- `bo list --branch <missing-branch>` reports no matching leaves without treating it as a tree error.
- `bo list --json` emits parseable JSON containing the same leaf data and degradation status as the human-readable output.
- Combined flags such as `bo list --branch <branch> --recent --limit 5 --json` behave consistently.
- Running `bo list` on an empty tree reports that no leaves have been collected.
- A leaf with missing or invalid metadata is shown with fallback display data and a visible degraded marker when possible.
- Running `bo list` does not create, modify, or delete tree files.
- The command does not require network access or an LLM/API key.

## Out of scope

- Listing branches as first-class rows.
- `bo show`, `bo status`, `bo search`, or `bo query`.
- Source/domain filtering.
- Full-text search or lexical ranking.
- Tree health diagnostics beyond row-level degraded markers.
- Repairing, pruning, rebuilding indices, or modifying metadata.
- Network fetching, revalidation, or refreshing collection metadata.
- Exact terminal table layout, column widths, colors, or symbols beyond requiring a visible degraded marker.

## Open questions

None for first scope.
