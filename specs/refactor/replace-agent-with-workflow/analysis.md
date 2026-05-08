# Analysis: Replace Compile Agent Loop with Structured-Output Pipeline

## Risk Assessment

### 1. Output truncation (HIGH)

`max_completion_tokens` is currently hardcoded to 4096. A compile response with 5 branches, each having a ~500-word markdown body, is ~4000 tokens. Larger collections (10+ branches) will exceed this, producing truncated JSON that fails to parse.

**Mitigation:** Set `max_completion_tokens` to 16384 (or remove the cap entirely and let the model decide). Detect truncation via `finish_reason: "length"` in the API response and surface a specific error: "compile output was truncated — try reducing collection size or using a model with larger output capacity."

### 2. async-openai API surface (LOW — now verified)

The research.md cited slightly wrong type names. Actual types in `async-openai 0.34`:

```rust
// src/types/shared/response_format.rs
pub enum ResponseFormat {
    Text,
    JsonObject,
    JsonSchema { json_schema: ResponseFormatJsonSchema },
}

pub struct ResponseFormatJsonSchema {
    pub description: Option<String>,
    pub name: String,
    pub schema: Option<serde_json::Value>,
    pub strict: Option<bool>,
}
```

The builder (`CreateChatCompletionRequestArgs`) supports `.response_format(ResponseFormat)`. No blockers here.

### 3. Prompt quality at launch (MEDIUM)

The prompt is marked "out of scope for quality tuning" but must be structurally correct on day one — the LLM must produce valid JSON conforming to the schema. With `strict: true`, the schema is guaranteed, but the *content quality* (meaningful concepts, correct leaf attributions) depends entirely on prompt design. A bad prompt means a technically working but useless compile.

**Mitigation:** Accept this as iteration risk. The pipeline is testable and the prompt is a const — easy to refine post-merge without code changes. First prompt should be clear and conservative.

## Gap Analysis

### 1. Prompt format unspecified

The plan says `build_compile_prompt()` but doesn't define how leaf content is serialized into the user message. Decisions needed:

- Delimiter format between leaves (e.g., `--- filename: foo.md ---\n{content}\n`)
- Whether to include full body or truncate long leaves
- Whether to include leaf metadata (title, URL) alongside body

**Recommendation:** Use a simple fenced format:
```
<document filename="understanding-ownership.md" title="Understanding Ownership">
{full markdown body}
</document>
```
This is unambiguous, parseable, and the LLM can reference filenames directly.

### 2. max_completion_tokens not addressed

The plan and tasks don't mention changing this from 4096. This will cause silent failures on any non-trivial collection.

**Recommendation:** Use 16384 as default. Expose via config if needed later.

### 3. "No patterns found" behavior ambiguous

Validation rule #1 says branches array "must be non-empty... or the LLM explicitly returns `{"branches": []}`." This is contradictory. What actually happens when the LLM finds no cross-cutting concepts?

**Recommendation:** `{"branches": []}` is valid. Pipeline still updates all leaf frontmatter with `branches: []` and prints "compiled: no branches found" (matching current summary behavior). Validation rule should be: branches array is always valid; individual branches within it must have non-empty title/body/leaves.

### 4. `finish_reason` not propagated

The `LlmProvider` trait returns `Result<String, LlmError>`. It doesn't propagate whether the response was truncated (`finish_reason: "length"`) or refused (`finish_reason: "content_filter"`). The caller has no way to distinguish a truncated JSON string from a genuine parse error.

**Recommendation:** Either:
- (a) Check `finish_reason` inside the provider and return `LlmError::Truncated` / `LlmError::Refused` variants, or
- (b) Return a richer type: `struct LlmResponse { content: String, finish_reason: FinishReason }`

Option (b) is cleaner and forward-compatible. The compile pipeline can then check `finish_reason` before attempting parse.

### 5. Leaf content reading not currently implemented

The current compile reads only frontmatter for validation (`leaf::read_frontmatter`). The new pipeline needs the **full markdown body** of every leaf. This is a new code path — `read_valid_leaves()` must read entire files, not just frontmatter.

**Recommendation:** Simple `fs::read_to_string` + `frontmatter::parse` to get both mapping and body. No new domain function needed.

## Edge Cases

### 1. Slug collisions from different titles

"Rust Ownership" and "Rust: Ownership" both slugify to `rust-ownership`. Validation must check uniqueness **post-slugification**, not at the title level.

### 2. Duplicate leaf within a single branch

The LLM might list the same leaf filename twice in one branch's `leaves` array. Should be deduplicated silently.

### 3. Leaf deleted between index read and frontmatter update

A leaf exists in `index.jsonl` but the file was deleted between `read_valid_leaves()` and `execute_plan()`. The frontmatter update will fail on `fs::read_to_string`. Current tool handles this gracefully (returns error string). New pipeline should warn and continue, not abort.

### 4. Branch body contains YAML frontmatter delimiters

If the LLM produces a body containing `---` on its own line, `domain::branch::write()` handles this correctly (it constructs frontmatter separately, not by string interpolation). No issue — but worth noting as a non-issue.

### 5. Empty branch body after heading

LLM returns `body: "# Title"` with no additional content. Is this valid? Current validation says "non-empty body" but doesn't define threshold. Should accept any non-whitespace body since the heading alone has marginal value — warn but don't reject.

### 6. Very large individual leaves

A single leaf might be 50K+ words (e.g., a book chapter transcript). This doesn't cause a schema issue but might push the prompt past context limits even with a small collection. The context-overflow error handles this, but the error message should hint at which leaf is the problem.

## Dependencies

### 1. async-openai 0.34 (CONFIRMED ✓)

Structured output support verified in source. No version bump needed.

### 2. OpenAI API structured output (CONFIRMED ✓)

GA since August 2024. All target models (gpt-4o, gpt-4o-mini, gpt-4.1) support it with `strict: true`.

### 3. No new crate dependencies

Everything needed (`serde_json`, `serde`, `async-openai`, `async-trait`, `tokio`, `chrono`) is already in Cargo.toml.

## Recommendation

**Ready to implement with four amendments to the plan:**

1. **Add `max_completion_tokens: 16384`** (or make it configurable) — current 4096 will fail on real collections.
2. **Define prompt format** — use XML-style document fencing as described above.
3. **Return `LlmResponse { content, finish_reason }` from provider** — enables truncation detection before JSON parse attempt.
4. **Clarify empty-branches behavior** — `{"branches": []}` is valid, triggers "compiled: no branches found" output + frontmatter reset on all leaves.

These are small additions to the existing tasks, not new tasks. With these addressed, the spec is tight and implementation can proceed.
