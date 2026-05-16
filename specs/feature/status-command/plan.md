# Plan: `bo status` — Tree Health at a Glance

## Architecture

### Tree infrastructure directory

This feature introduces `{tree}/.bo/` as the home for all tree-local operational metadata. Existing trees have `index.jsonl` at the tree root; this is migrated into `.bo/` via an auto-migration mechanism.

Files in `{tree}/.bo/`:
- `version` — single integer, current tree schema version
- `index.jsonl` — leaf navigation/dedup cache (moved from tree root)
- `state.json` — compile tracking (new)

### Auto-migration

A new `engine::migrate` module runs on every tree-touching command, after config load, before any reads. It:

1. Reads `{tree}/.bo/version` (absent = version 0)
2. Compares against the version bo understands (currently: 1)
3. Runs migrations sequentially (0→1, future: 1→2, etc.)
4. If tree version > bo's version: exit with "tree created by newer bo — please upgrade"

Migration 0→1:
1. Create `{tree}/.bo/`
2. Move `{tree}/index.jsonl` → `{tree}/.bo/index.jsonl`
3. Write `{tree}/.bo/version` with contents `1`

Idempotent: each step checks preconditions. Partial failures are safe — next run completes the migration.

### Status pipeline

```
config load
  → migrate (ensure tree at current version)
  → read index ({tree}/.bo/index.jsonl)
  → read state ({tree}/.bo/state.json, absent = empty)
  → scan filesystem (leaves at tree root, branches at {tree}/branches/)
  → compute health (orphans, missing entries)
  → derive hints (actionable next steps)
  → format output (human or JSON)
```

Deterministic, read-only (status itself writes nothing), no LLM calls.

### Key components

| Component | Location | Responsibility |
|-----------|----------|---------------|
| `engine::migrate` | `src/engine/migrate.rs` | Version detection, migration runner, individual migration functions |
| `engine::state` | `src/engine/state.rs` | Read/write `state.json` (compiled leaf set + timestamps) |
| `cli::status` | `src/cli/status.rs` | Orchestrates the status pipeline, health checks, output formatting |
| CLI wiring | `src/main.rs` | Add `Status` variant to `Commands` enum, dispatch |

### Integration points

- **`engine::config`** — read tree config to get `output_dir`
- **`domain::index`** — read index entries (path updated to `.bo/index.jsonl`)
- **`engine::state`** — read compile state
- **`domain::branch`** — read branch `compiled_at` frontmatter for last compile time
- **All tree-touching commands** — must call `engine::migrate::ensure_current()` before accessing tree files

### Compile integration

After a successful compile, `cli::compile` writes to `state.json`:
- Updates `compiled_leaves` with the slugs processed in this run
- Updates `last_compiled_at` timestamp

This is a small addition to the existing compile pipeline (append to state after branch writes succeed).

## Implementation strategy

### Order of work

1. **`engine::migrate`** — version detection + migration 0→1 (move index into .bo/)
2. **Update all tree-touching commands** — call `ensure_current()` early in their pipeline, update index path references from `{tree}/index.jsonl` to `{tree}/.bo/index.jsonl`
3. **`engine::state`** — state.json read/write module
4. **Compile integration** — write compiled leaf slugs to state after successful compile
5. **`cli::status`** — the status command itself (health checks, formatting, hints)
6. **CLI wiring** — add subcommand to main.rs
7. **Tests** — unit tests per module, integration test for the full flow

### Index path migration

All code currently referencing `tree_dir.join("index.jsonl")` must update to `tree_dir.join(".bo/index.jsonl")`. This is a mechanical find-and-replace across:
- `cli::collect` (append entry)
- `cli::compile` (read entries)
- `cli::list` (read entries)
- `cli::search` (read entries)
- `cli::query` (read entries)
- `cli::seed` (initial empty index? — check)

A helper on `Tree` or a free function `index_path(tree_dir) -> PathBuf` centralises this.

### Health checks

**Orphan detection:**
```
for entry in index:
    if !tree_dir.join(entry.file).exists():
        orphans.push(entry)
```

**Missing detection:**
```
leaf_files = glob {tree_dir}/*.md
indexed_files = index.entries.map(|e| e.file).collect::<HashSet>()
for file in leaf_files:
    if file not in indexed_files:
        missing.push(file)
```

### Hint generation

Hints are deterministic rules:
- Uncompiled leaves > 0 → `"run 'bo compile' to process N new leaves"`
- Orphan entries > 0 → `"N index entries reference missing files — re-collect or remove manually"`
- Missing entries > 0 → `"N leaf files are not indexed — they won't appear in search or compile"`
- No branches exist → `"run 'bo compile' to create your first branch"`
- No leaves exist → `"run 'bo collect <url>' to add your first source"`

### Human output format

```
bo · my-research

  Leaves:     12 (3 uncompiled)
  Branches:    4
  Last compile: 2026-05-14T20:54:55Z
  Size:       48 KB (~12,000 tokens)

  Uncompiled:
    • agentic-coding-is-a-trap-lars-faye
    • has-the-data-center-boom-hit-a-wall
    • the-ai-industry-is-lying-to-you

  Hints:
    → run 'bo compile' to process 3 new leaves
```

Health issues (if any) appear between Size and Hints with yellow/warning styling if terminal supports it.

### JSON output shape

```json
{
  "tree_name": "my-research",
  "leaves": {
    "total": 12,
    "uncompiled": 3,
    "uncompiled_slugs": ["slug-a", "slug-b", "slug-c"]
  },
  "branches": {
    "total": 4,
    "last_compiled_at": "2026-05-14T20:54:55Z"
  },
  "size": {
    "bytes": 49152,
    "estimated_tokens": 12288
  },
  "health": {
    "orphan_index_entries": [],
    "missing_from_index": []
  },
  "hints": [
    "run 'bo compile' to process 3 new leaves"
  ]
}
```

### Error handling

| Condition | Behaviour |
|-----------|-----------|
| Not seeded | Exit 2, standard not-seeded message |
| Tree dir doesn't exist | Exit 2, "tree directory not found: {path}" |
| Migration failure (permissions, etc.) | Exit 2, surface I/O error |
| Tree version > bo's version | Exit 2, "tree requires bo version X — please upgrade" |
| state.json absent | Treat as empty (all leaves uncompiled) — not an error |
| state.json malformed | Warning to stderr, treat as empty, continue |
| index.jsonl absent | 0 leaves, hint to collect |

### External dependencies

None new. Uses: `serde`, `serde_json`, `chrono`, `std::fs` — all already in the dependency tree.
