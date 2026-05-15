# Spec: `bo status` — Tree Health at a Glance

## Problem statement

After collecting leaves and compiling branches, there is no way to inspect the state of a tree without manually counting files or reading frontmatter. Users and agents cannot answer basic questions: How many leaves do I have? Which ones haven't been compiled yet? Is the tree healthy? What should I do next?

This gap blocks incremental compile (which needs to know what's new) and makes bo opaque to both humans and programmatic callers.

## User-facing requirements

1. **Basic invocation**: `bo status` prints a summary of the active tree's health and readiness.

2. **Leaf count and branch count**: total number of leaves and branches currently in the tree.

3. **Uncompiled leaves**: leaves that have never been compiled. Tracked via a dedicated tree state file (`{tree}/.bo/state.json`) that records which leaf slugs have been compiled and when. The index remains a pure navigation/dedup cache (rebuildable from leaf frontmatter). Leaf files are never modified by status or compile tracking. A leaf is "uncompiled" if its slug appears in the index but not in the state file's compiled set.

4. **Tree size**: total bytes on disk (leaves + branches) and an estimated token count (bytes ÷ 4 heuristic).

5. **Last compile time**: the most recent `compiled_at` timestamp across all branch files. Absent if no branches exist.

6. **Index health**: detect and report:
   - **Orphan index entries** — index references a file that doesn't exist on disk.
   - **Missing index entries** — leaf files on disk that have no corresponding index entry.
   
   Each issue includes the specific files affected and a suggested remediation (e.g. "re-collect the URL" or "run `bo collect` to re-index").

7. **Human output**: a compact summary table/block. Uncompiled leaves listed by slug (capped at a reasonable display limit with "and N more…" overflow). Health issues listed individually with remediation hints.

8. **`--json` output**: structured JSON containing all data points above, plus a `hints` array of actionable next-step suggestions (e.g. `"run 'bo compile' to process 3 new leaves"`).

9. **Read-only**: the command never modifies the tree, index, config, or any file.

10. **Exit codes**: always exits 0 if the tree is seeded and status was successfully determined. Exits non-zero only if the command itself fails (not seeded, I/O error, corrupt config). Tree health problems are reported in output, not via exit code.

## Success criteria

- After collecting 3 new leaves without compiling: `bo status` shows them as uncompiled and hints at running compile.
- After a full compile: `bo status` shows 0 uncompiled leaves and reports the compile timestamp.
- Manually deleting a leaf file: `bo status` reports the orphan index entry and suggests remediation.
- A leaf file present on disk but missing from the index: reported with remediation.
- `bo status --json | jq` produces valid, parseable JSON with all fields populated.
- An agent can consume `--json` output to decide whether to invoke `bo compile`.

## Out of scope

- Modifying the index to fix health issues (that's a future `bo doctor` or `bo reindex`).
- Branch staleness detection (branches referencing deleted leaves) — deferred to incremental compile.
- Token-accurate counting (tiktoken or similar) — heuristic is sufficient.
- Tree history or changelog of operations.
- Writing `last_compiled_at` to the index — that's compile's job. Status only reads it.

## Open questions

None.
