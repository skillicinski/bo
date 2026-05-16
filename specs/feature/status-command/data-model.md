# Data Model: `bo status`

## State file — `{tree}/.bo/state.json`

Compile tracking state. Written by `bo compile` after successful runs. Read by `bo status`. Absent until first compile.

```json
{
  "compiled_leaves": {
    "slug-a": "2026-05-14T20:54:55Z",
    "slug-b": "2026-05-14T20:54:55Z",
    "slug-c": "2026-05-15T10:00:00Z"
  }
}
```

### Fields

| Field | Type | Description |
|-------|------|-------------|
| `compiled_leaves` | `Map<String, String>` | Leaf slug → ISO 8601 timestamp of when it was last included in a compile run |

### Design notes

- Map rather than array so lookups are O(1) when checking "is this leaf compiled?"
- Timestamp per leaf supports future incremental compile: "recompile leaves compiled before X"
- No `last_compiled_at` top-level field — derivable from `max(compiled_leaves.values())` or from branch frontmatter
- Absent file = no leaves compiled (all are new)
- Malformed file = warning + treat as empty (graceful degradation)

---

## Version file — `{tree}/.bo/version`

Plain text file containing a single integer. No trailing newline required (but tolerated).

```
1
```

### Version semantics

| Version | Layout |
|---------|--------|
| 0 (implicit — no `.bo/` dir) | `index.jsonl` at tree root, no state file |
| 1 | `{tree}/.bo/index.jsonl`, `{tree}/.bo/state.json`, `{tree}/.bo/version` |

---

## Status output — `StatusResult` struct

The internal representation produced by the status pipeline, consumed by both human and JSON formatters.

```rust
pub struct StatusResult {
    pub tree_name: String,
    pub leaves_total: usize,
    pub leaves_uncompiled: Vec<String>,  // slugs
    pub branches_total: usize,
    pub last_compiled_at: Option<String>, // ISO 8601, from branch frontmatter
    pub size_bytes: u64,
    pub estimated_tokens: u64,           // size_bytes / 4
    pub health: HealthReport,
    pub hints: Vec<String>,
}

pub struct HealthReport {
    pub orphan_index_entries: Vec<OrphanEntry>,
    pub missing_from_index: Vec<String>,  // filenames
}

pub struct OrphanEntry {
    pub file: String,
    pub title: String,
    pub url: String,
    pub remediation: String,
}
```

---

## Index entry — unchanged

`IndexEntry` in `domain::index` keeps its existing shape:

```rust
pub struct IndexEntry {
    pub file: String,
    pub title: String,
    pub url: String,
}
```

Only its file path changes (tree root → `.bo/`). No structural modification.

---

## Relationships

```
{tree}/.bo/version      ← checked by engine::migrate on every command
{tree}/.bo/index.jsonl  ← navigation cache, source of leaf inventory
{tree}/.bo/state.json   ← compile tracking, source of "what's been compiled"
{tree}/branches/*.md    ← source of last_compiled_at (from frontmatter)
{tree}/*.md             ← actual leaf files on disk (for health checks)
```

Uncompiled leaves = `index slugs − state.compiled_leaves.keys()`
Orphans = `index entries where file doesn't exist on disk`
Missing = `leaf .md files on disk not referenced by any index entry`
