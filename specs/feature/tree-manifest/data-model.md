# Data Model: Tree Manifest (3a)

## Storage

- **Location:** `{tree}/.bo/manifest.json`
- **Format:** JSON, pretty-printed (`serde_json::to_string_pretty`) for diffability and human inspection.
- **Atomicity:** writes go to `manifest.json.tmp`, fsync, then `rename` to `manifest.json`. POSIX rename guarantees atomic replacement on a single filesystem.
- **Single source of truth:** for tree topology and per-entity metadata. Content of leaves and branches lives in their respective `.md` files.

## Entities

### `Manifest` (root)

| Field | Type | Notes |
|---|---|---|
| `tree` | `TreeMeta` | Tree-level metadata. |
| `leaves` | `Vec<LeafRecord>` | All collected leaves, in collection order. |
| `branches` | `Vec<BranchRecord>` | All compiled branches. |

Iteration order of `leaves` and `branches` is preserved across reads/writes; consumers that need stable ordering can rely on insertion order.

### `TreeMeta`

| Field | Type | Notes |
|---|---|---|
| `name` | `String` | Human-readable tree name; matches `TreeConfig.name`. |
| `created_at` | `String` | ISO 8601 UTC timestamp from `bo seed`. |
| `last_compiled_at` | `Option<String>` | ISO 8601 UTC; set on successful compile. `None` until first compile. |

### `LeafRecord`

| Field | Type | Notes |
|---|---|---|
| `slug` | `String` | Stable identifier; matches the leaf's `.md` basename without extension. |
| `file` | `String` | Filename relative to tree root, e.g. `my-leaf.md`. |
| `title` | `String` | Display title; matches frontmatter. |
| `url` | `String` | Source URL (or canonical equivalent for non-URL adapters). |
| `collected_at` | `String` | ISO 8601 UTC. |
| `summary` | `Option<String>` | LLM-generated summary; `None` if extraction failed or summary disabled. `#[serde(default)]`. |

No back-reference to branches. Which branches a leaf is part of is computed at call time by `Manifest::branches_for_leaf`.

### `BranchRecord`

| Field | Type | Notes |
|---|---|---|
| `slug` | `String` | Stable identifier; matches the branch's `.md` basename without extension. |
| `file` | `String` | Filename relative to tree root, e.g. `branches/topic.md`. |
| `title` | `String` | Display title; matches frontmatter. |
| `created_at` | `String` | ISO 8601 UTC of the compile run that **first produced** this branch. Preserved across recompiles. |
| `updated_at` | `String` | ISO 8601 UTC of the **most recent** compile run that touched this branch. Updated on every recompile. |
| `stale` | `bool` | Always `false` in 3a. Field is reserved for incremental compile (item 4). `#[serde(default)]`. |
| `leaves` | `Vec<String>` | Slugs of leaves assigned to this branch. **Canonical** direction of the cross-reference. |

**Note on naming.** The corresponding fields in branch `.md` frontmatter are renamed in lockstep: `compiled_at` → `created_at`, `updated_at` stays. The manifest and the frontmatter use identical names. This is a breaking change to file format, accepted because v0.0.2 is a fresh-tree release.


## Relationships

```
Manifest
  ├── tree: TreeMeta            (1:1)
  ├── leaves: Vec<LeafRecord>   (1:N, ordered by collected_at)
  └── branches: Vec<BranchRecord> (1:N, ordered by compiled_at)

BranchRecord ──canonically references──→ LeafRecord (by slug)

LeafRecord  ←─derived in-memory─── BranchRecord (Manifest::branches_for_leaf)
```

The branch-to-leaf reference is unidirectional on disk. The inverse is computed by scanning `manifest.branches` and collecting any branch whose `leaves` vector contains the target slug. This is O(B × L) per call where B = branch count, L = avg leaves per branch — irrelevant at target scale (<1000 leaves, <50 branches).

A leaf is **uncompiled** iff `tree.last_compiled_at` is `None` or `leaf.collected_at > tree.last_compiled_at` — i.e., no compile pass has run since this leaf was added. `Manifest::uncompiled_leaves` materializes this set in a single pass.

**Behavioural clarification from v0.0.1.** Previously, leaves with unparseable frontmatter were excluded from `state.compiled_leaves` and consequently shown as "uncompiled" in status. Under the manifest model they are no longer counted as uncompiled (the compile pass has run, time has moved forward); the existing health check continues to flag them separately as `skipped`. This disambiguates "never seen" from "seen but broken."

A branch is **stale** iff its `stale` field is `true`. In 3a this is never set; the field exists so the on-disk schema is forward-compatible with item 4 (incremental compile + deletion-detection). No migration burden when item 4 ships.

## Invariants

The following must hold for any manifest written by bo. Violations indicate a bug.

1. **Slug uniqueness within type.** No two `LeafRecord`s share a slug; no two `BranchRecord`s share a slug. Enforced by `debug_assert!` in `manifest::write` (panic in debug builds, no-op in release).
2. **Slug uniqueness across types.** A leaf slug never collides with a branch slug. Enforced by branches living under `branches/` while leaves live at the tree root.
3. **Cross-reference integrity.** Every slug in any `BranchRecord.leaves` resolves to an entry in `manifest.leaves`. Compile ensures this; if a future operation deletes a leaf without updating branches, the dangling reference is what `BranchRecord.stale` is meant to flag (item 4).
4. **File-field correctness.** `LeafRecord.file` and `BranchRecord.file` resolve to existing files on disk after a successful mutating operation. Recovery from drift is 3b's concern.
5. **Timestamp monotonicity per branch.** `BranchRecord.created_at` for a given slug never moves across writes (preserved). `BranchRecord.updated_at` is monotonically non-decreasing across writes. Not enforced by code in 3a; documented as an expectation.

## On-Disk Example

```json
{
  "tree": {
    "name": "rust-notes",
    "created_at": "2026-05-19T14:00:00Z",
    "last_compiled_at": "2026-05-19T14:32:11Z"
  },
  "leaves": [
    {
      "slug": "ownership-and-borrowing",
      "file": "ownership-and-borrowing.md",
      "title": "Ownership and Borrowing",
      "url": "https://doc.rust-lang.org/book/ch04-00-understanding-ownership.html",
      "collected_at": "2026-05-19T14:05:32Z",
      "summary": "Rust's ownership rules and how borrowing enables safe references."
    }
  ],
  "branches": [
    {
      "slug": "memory-safety",
      "file": "branches/memory-safety.md",
      "title": "Memory Safety",
      "created_at": "2026-05-19T14:32:11Z",
      "updated_at": "2026-05-19T14:32:11Z",
      "stale": false,
      "leaves": ["ownership-and-borrowing"]
    }
  ]
}
```

## Reconstruction (transient affordance)

When `manifest::read` finds the file absent but the secondary store present, it rebuilds a manifest by:

1. Reading the secondary leaf roster → `LeafRecord` entries (slug from filename; title/URL from frontmatter; collected_at from frontmatter; summary from frontmatter if present).
2. Walking the branches directory → `BranchRecord` entries (slug from filename; title/created_at/updated_at/leaves from frontmatter).
3. `tree.last_compiled_at = max(BranchRecord.updated_at)` across reconstructed branches, or `None` if no branches exist.
4. `tree.name` and `tree.created_at` come from the existing tree config (`~/.bo/config.json`); fall back to directory basename / current time with a stderr warning if absent.
5. The reconstructed manifest is written to disk so subsequent reads bypass this path.

**Parse errors do NOT trigger reconstruction.** If `manifest.json` exists but is invalid JSON, `manifest::read` surfaces the parse error. Reconstruction runs only when the file is absent. Auto-reconstructing over a corrupt file would silently destroy whatever the user broke.

This logic exists only because the secondary store is still being written during 3a. It is removed in 3b along with the secondary writers.
