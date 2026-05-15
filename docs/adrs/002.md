# ADR-002: Structured Output Schema Guidelines

**Status:** Accepted
**Date:** 2026-05-10

---

## Context

Bo produces structured output in two directions:

1. **CLI → consumers** (`--json` flag): agents, scripts, and MCP clients parse bo's output programmatically.
2. **Bo → LLM** (structured-output calls): compile, query, and future commands constrain LLM responses via JSON schemas.

Both directions benefit from consistent schema design principles. Without guidelines, schemas accumulate inconsistencies — nested optional objects, stringly-typed enums, missing required fields — that make both LLM compliance and downstream parsing fragile.

---

## Decision

All JSON schemas in bo (CLI output and LLM structured-output constraints) follow these principles:

### Flatness

Prefer flat objects over deeply nested structures. One level of nesting for arrays of objects is acceptable; avoid deeper hierarchies unless the domain demands it.

```json
// prefer
{ "title": "...", "url": "...", "score": 0.8 }

// avoid
{ "metadata": { "source": { "title": "...", "url": "..." } }, "ranking": { "score": 0.8 } }
```

### Strong typing

Use the most specific JSON Schema type available:

- `"type": "string"` for text
- `"type": "number"` or `"type": "integer"` for numeric values
- `"type": "boolean"` for flags
- `"type": "array"` with typed `items` for lists
- `"type": "object"` with explicit `properties` for structured records
- `"enum"` for constrained string values (status codes, categories, modes)

Never use `"type": "string"` where an enum or number is semantically correct.

### Required fields

All fields that must be present for the output to be meaningful are listed in `"required"`. Optional fields are the exception, not the default. For LLM structured-output schemas, prefer all-required to reduce omission errors.

### Semantic descriptors

Field names are self-documenting. Use `"description"` in schemas passed to LLMs to clarify intent when the field name alone is ambiguous.

```json
{
  "properties": {
    "relevance": {
      "type": "string",
      "enum": ["high", "medium", "low"],
      "description": "How relevant this leaf is to the user's query"
    }
  }
}
```

### Modularity

Complex schemas compose small, reusable sub-schemas. Define a leaf citation shape once and reference it in query output, compile output, and search output.

### No `additionalProperties`

Set `"additionalProperties": false` on all object schemas passed to LLMs. This prevents models from inventing extra fields and keeps output predictable.

---

## Consequences

**Positive:**

- LLM structured-output compliance improves (simpler schemas → fewer parse failures)
- `--json` output is consistent across commands — agents learn one style
- Schemas serve as de-facto documentation for MCP tool contracts
- Type-safe deserialization in Rust (serde) maps cleanly to flat required-field schemas

**Negative:**

- Some domain complexity requires flattening decisions that feel unnatural
- All-required fields can produce verbose output when some data is genuinely absent (mitigated: use sentinel values or nullable types explicitly)

---

## References

- OpenAI Structured Outputs documentation: flat schemas with all-required fields maximize compliance
- `src/cli/json.rs`: existing bo JSON rendering module
- ADR-001: structured-output LLM calls as the standard pattern
