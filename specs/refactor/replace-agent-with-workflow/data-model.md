# Data Model: Compile Response Schema

## Compile Response (LLM → pipeline)

The single structured-output LLM call returns JSON conforming to this schema. This is the contract between the prompt and the pipeline's parse/validate step.

### Schema

```json
{
  "type": "object",
  "properties": {
    "branches": {
      "type": "array",
      "items": {
        "type": "object",
        "properties": {
          "title": {
            "type": "string",
            "description": "Human-readable concept name (e.g. 'Rust Ownership')"
          },
          "body": {
            "type": "string",
            "description": "Markdown body describing the concept across the collection. Should begin with a heading matching the title."
          },
          "leaves": {
            "type": "array",
            "items": { "type": "string" },
            "description": "Filenames (with .md) of leaves this concept appears in"
          }
        },
        "required": ["title", "body", "leaves"],
        "additionalProperties": false
      }
    }
  },
  "required": ["branches"],
  "additionalProperties": false
}
```

### Example Response

```json
{
  "branches": [
    {
      "title": "Rust Ownership",
      "body": "# Rust Ownership\n\nOwnership appears across multiple documents in this collection...",
      "leaves": ["understanding-ownership.md", "borrowing-patterns.md", "memory-safety.md"]
    },
    {
      "title": "Zero-Cost Abstractions",
      "body": "# Zero-Cost Abstractions\n\nSeveral documents discuss the principle of...",
      "leaves": ["understanding-ownership.md", "trait-objects.md"]
    }
  ]
}
```

### Derived Data

The pipeline derives leaf→branch assignments by inverting the branches array:

```
For each branch B:
  For each leaf filename in B.leaves:
    leaf_assignments[filename].push(B.slug)
```

This produces the `branches: [slug-a, slug-b]` list written into each leaf's frontmatter. Leaves not referenced by any branch get `branches: []`.

### Validation Rules (applied before any writes)

1. `branches` array must be non-empty (at least one concept found) — or the LLM explicitly returns `{"branches": []}` indicating no patterns found.
2. Each branch must have a non-empty `title`, non-empty `body`, and non-empty `leaves` array.
3. Every filename in `leaves` must exist in the known valid filenames set (from index). Unknown filenames are filtered with a warning, not a hard failure.
4. Branch slugs (derived from title via `domain::slug::slugify`) must be unique. Duplicate titles are a validation error.

### Unchanged On-Disk Formats

The pipeline writes using existing domain functions. On-disk formats are unchanged:

**Branch file** (`branches/{slug}.md`):
```yaml
---
title: Rust Ownership
compiled_at: 2026-05-08T10:00:00Z
updated_at: 2026-05-08T10:00:00Z
leaves:
  - understanding-ownership.md
  - borrowing-patterns.md
---

# Rust Ownership

Body content...
```

**Leaf frontmatter patch** (added/updated fields only):
```yaml
updated_at: 2026-05-08T10:00:00Z
branches:
  - rust-ownership
  - zero-cost-abstractions
```
