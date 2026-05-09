# Data Model: `bo search`

## Input types

### SearchQuery

```rust
pub struct SearchQuery {
    /// Each entry is a term or phrase (quoted args become single entries).
    /// All lowercased at parse time.
    pub terms: Vec<String>,
}
```

Constructed directly from clap's `Vec<String>` positional args. Each arg is one term/phrase, already split by the shell.

### SearchOptions

```rust
pub struct SearchOptions {
    pub page: usize,        // 1-indexed, default 1
    pub recent: bool,       // sort by collected_at instead of relevance
    pub json: bool,         // output format flag (handled at render layer)
}
```

## Output types

### SearchResult

```rust
#[derive(Serialize)]
pub struct SearchResult {
    pub hits: Vec<SearchHit>,
    pub total_results: usize,
    pub page: usize,
    pub total_pages: usize,
}
```

### SearchHit

```rust
#[derive(Serialize)]
pub struct SearchHit {
    pub file: String,           // relative path from tree root
    pub title: String,          // from frontmatter, fallback to index title → filename
    pub snippet: String,        // KWIC ±80 chars from body, or first 160 chars fallback
    pub score: usize,           // per-mille density: (occurrences * 1000) / content.len()
    pub collected_at: Option<String>,  // for --recent sorting
}
```

## Internal (not serialized)

### ScoredLeaf (intermediate, pre-pagination)

```rust
struct ScoredLeaf {
    file: String,
    title: String,
    body: String,               // needed for snippet extraction
    score: usize,
    collected_at: Option<String>,
    index_position: usize,      // tie-breaker
}
```

## Storage

No new storage. Search reads:
- `{tree_dir}/index.jsonl` — file paths to enumerate
- `{tree_dir}/{file}` — full leaf content (frontmatter + body)

Nothing is written.

## Constants

```rust
const PAGE_SIZE: usize = 5;
const SNIPPET_RADIUS: usize = 80;      // chars on each side of match
const FALLBACK_SNIPPET_LEN: usize = 160; // chars from body start when match is frontmatter-only
```
