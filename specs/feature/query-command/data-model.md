# Data Model: `bo query` V1

## Core entities

### QueryResult

The output of a successful query pipeline run.

```rust
pub struct QueryResult {
    pub answer: String,           // Prose with [[slug]] citations inline
    pub citations: Vec<Citation>, // Metadata for each cited leaf
    pub model: String,            // Model used for synthesis
    pub leaves_consulted: usize,  // Number of leaves whose full body was sent to LLM
}
```

### Citation

A single cited source leaf.

```rust
pub struct Citation {
    pub slug: String,   // Leaf slug (filename without .md)
    pub title: String,  // Leaf title from frontmatter
    pub file: String,   // Relative path to leaf file
}
```

### RetrievedLeaf

Internal — a leaf scored and loaded for potential inclusion in context.

```rust
struct RetrievedLeaf {
    pub slug: String,
    pub title: String,
    pub url: String,
    pub file: String,       // Relative path
    pub summary: String,    // From frontmatter (or empty)
    pub body: String,       // Full markdown body
    pub score: f64,         // Term density score (OR semantics)
}
```

### SynthesisResponse

Deserialized from the LLM structured output.

```rust
#[derive(Deserialize)]
struct SynthesisResponse {
    pub answer: String,
    pub cited_slugs: Vec<String>,
}
```

## Config extension

```rust
pub struct Config {
    pub tree: TreeConfig,
    pub compile_model: Option<String>,
    pub query_model: Option<String>,  // NEW — defaults to "gpt-4o"
}

impl Config {
    pub fn effective_query_model(&self) -> &str {
        self.query_model.as_deref().unwrap_or("gpt-4o")
    }
}
```

## Storage

No new persistent storage. Query is read-only and stateless. All data is read from:
- `index.jsonl` — leaf enumeration
- `leaves/*.md` — frontmatter + body content
- `~/.bo/config.json` — model config

## JSON output schema (ADR-002 compliant)

```json
{
  "type": "object",
  "properties": {
    "answer": { "type": "string" },
    "citations": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "slug": { "type": "string" },
          "title": { "type": "string" },
          "file": { "type": "string" }
        },
        "required": ["slug", "title", "file"],
        "additionalProperties": false
      }
    },
    "model": { "type": "string" },
    "leaves_consulted": { "type": "integer" }
  },
  "required": ["answer", "citations", "model", "leaves_consulted"],
  "additionalProperties": false
}
```
