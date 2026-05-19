# Tree Manifest (3a): Reads + Dual-Write

**Branch:** `feature/tree-manifest`
**Milestone:** v0.0.2 item 3a
**ADR:** [ADR-004](../../../docs/adrs/004.md)

## Problem Statement

Tree state is currently fragmented across `index.jsonl` (leaf roster), `state.json` (compile timestamp), filesystem scans of `branches/`, and per-file frontmatter. Every command that needs to know "what's in this tree" reconstructs that view differently. There is no single source of truth.

This fragmentation is the root cause of three v0.0.2 goals being hard or impossible:

- **Incremental compile** needs to know which leaves are uncompiled and which branches are stale — currently requires scanning the branches directory and parsing every branch's frontmatter.
- **Crash safety** is impossible without a single commit point; today, a compile that writes 3 branch files and updates state can fail mid-way and leave the tree inconsistent.
- **`bo status`** answers some questions from `index.jsonl` and others from filesystem walks, with the two views allowed to disagree.

This spec covers stage **3a**: introduce the manifest as the canonical *read* surface while continuing to write legacy files in parallel. It is a behaviour-preserving refactor with one new artifact on disk. Crash-safety mechanisms (`pending.json`, advisory lock, recovery) are out of scope and ship in 3b.

## User-Facing Requirements

1. **`bo seed` produces a `manifest.json`.**
   After `bo seed <dir>`, `<dir>/.bo/manifest.json` exists alongside the existing seed artifacts. It contains the tree's name and creation timestamp, with empty leaves and branches.

2. **`bo collect` updates the manifest.**
   After `bo collect <url>`, the new leaf appears in `manifest.json` with its slug, file, title, URL, collected_at, and (if extracted) summary. The existing `index.jsonl` continues to be appended in parallel — no behaviour change visible to the user.

3. **`bo compile` updates the manifest.**
   After a successful compile, all branch records exist in `manifest.json` with their slug, file, title, compiled_at, and the leaf slugs they contain. `tree.last_compiled_at` is set. The existing `state.json` continues to be written in parallel.

4. **All read commands consult the manifest.**
   `bo status`, `bo list`, `bo show`, `bo query`, and duplicate-detection inside `bo collect` answer all questions about tree topology from `manifest.json`. They no longer read `index.jsonl`, `state.json`, or scan the `branches/` directory for metadata.

5. **Reads survive deletion of legacy files.**
   With `manifest.json` present, manually deleting `index.jsonl` and `state.json` does not break any read command. Output is identical to the pre-deletion run. (This proves the read paths have actually moved.)

6. **`bo raze` removes the manifest.**
   `bo raze` deletes `manifest.json` along with the existing tree artifacts.

7. **No user-visible behaviour changes.**
   Output of every command — human and `--json` — is byte-equivalent to pre-3a behaviour for any tree built from scratch on this version. No new flags, no new error codes, no new prompts.

## Success Criteria

- `bo seed /tmp/t && cat /tmp/t/.bo/manifest.json` shows valid JSON with the tree's name, `created_at`, `last_compiled_at: null`, empty leaves, empty branches.
- `bo seed /tmp/t && bo collect <url>` produces a manifest entry whose `slug`, `url`, `title`, and `file` match the leaf's frontmatter and the corresponding `index.jsonl` line.
- `bo compile` against a seeded tree with N leaves produces a manifest with M branch records (M ≥ 1 in a non-empty tree), each listing the leaves it contains, and `tree.last_compiled_at` set.
- After `bo compile`, deleting `.bo/index.jsonl` and `.bo/state.json` and running `bo status`, `bo list`, `bo show <slug>`, `bo query "..."` produces output identical to runs with those files present.
- A snapshot test of `bo status --json` and `bo list --json` against a fixed fixture tree returns the same bytes before and after the 3a changes.
- `bo raze` on a 3a tree leaves no `manifest.json`, `index.jsonl`, `state.json`, or `branches/` behind.
- Unit tests cover: manifest read/write round-trip, dual-write parity (manifest entries match `index.jsonl` entries after a sequence of collects), resolution helpers (`leaf_by_slug`, `branch_by_slug`, `uncompiled_leaves`, `branches_for_leaf`).

## Out of Scope

- **Crash safety / `pending.json` / advisory lock / recovery** — 3b.
- **Removing legacy writers** (`index.jsonl`, `state.json`, `src/engine/state.rs`, `src/domain/index.rs`) — 3b.
- **Concurrent-process refusal messaging** (exit 2, "another bo process is already interacting with…") — 3b.
- **Recovery messaging** (one-liner on interrupted-op detection) — 3b.
- **Migration from pre-manifest trees.** No such trees exist in the wild; not handled.
- **Schema versioning.** ADR-004 explicitly defers this.
- **Modification-based staleness detection.** Out of scope per ADR-004; 3a only tracks `stale: bool` as a field in `BranchRecord`, never sets it.
- **Incremental compile.** Item 4. The manifest enables it but compile still rebuilds the full corpus in 3a.
- **`bo status` displaying new manifest-derived fields** beyond the existing v0.0.2 item 1 surface.

## Open Questions

None — all design decisions resolved in ADR-004 and the preceding audit.
