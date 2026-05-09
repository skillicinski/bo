# Analysis: Leaf Summary

## Risk assessment

### Low risk
- **Deterministic fallback** — pure string splitting, trivial to implement and test.
- **Backward compatibility** — `summary:` is additive. Existing leaves without it continue working. `frontmatter::parse` returns a `Mapping` where absent keys are just `None`.
- **Provider reuse** — same `LlmProvider` trait, same `OpenAiProvider`, same tokio pattern as compile. No new infrastructure.

### Medium risk
- **YAML block scalar formatting** — `leaf::write` uses manual string construction (not serde_yaml serialization). Inserting a `summary: |` block with indented lines must handle: trailing newlines, empty lines within the summary, and the literal block scalar requiring a final newline. Getting the indentation wrong produces invalid YAML.
- **`leaf::write` signature change** — touches one production caller (`collect.rs`) and 5 test call sites in `leaf.rs`. Mechanical but needs care to not break existing tests.
- **Model cost/speed assumption** — the plan says "cheap/fast model" but uses `effective_compile_model()` which defaults to `gpt-4o`. That's neither cheap nor fast for per-leaf summarization. Works for dogfood, but users collecting 50 URLs would burn tokens unnecessarily.

### Low-but-notable risk
- **LLM response quality** — structured output with a simple schema should be reliable, but the LLM could return a summary that's too short, too long, or includes meta-commentary despite the prompt. Not a correctness issue (it's still stored), just a quality variance.

## Gap analysis

### 1. Model selection — using compile_model is expensive for summarization

The spec says "reuse existing provider config" and the plan uses `effective_compile_model()` (defaults to `gpt-4o`). For per-leaf summarization of potentially many URLs, gpt-4o is overkill. The spec acknowledges this ("cheap/fast model") but doesn't resolve it.

**Decision:** accept `gpt-4o` as the default for MVP. The future `summary_model` config field will fix this. Document in implementation that this is a known cost concern.

### 2. YAML block scalar edge cases

What happens when the summary:
- Contains a line that starts with `---` (YAML document separator)?
- Is exactly one line (no newlines)?
- Contains trailing whitespace on lines?
- Is empty string?

**Decision:** for single-line summaries (likely from fallback for short bodies), use a plain quoted scalar. For multi-line, use `|` block scalar. The `format_content` function already handles quoting for title — same pattern applies.

Actually, revisiting: the fallback is "first 200 words joined with spaces" — that's always a single line. Only LLM summaries might be multi-line (if the model inserts line breaks). Simpler approach: **always store summary as a double-quoted scalar** (same as title), escaping internal quotes/newlines. This avoids all block scalar edge cases.

Wait — double-quoted scalars with 200 words would produce an unreadably long line in the raw file. The plan explicitly chose `|` for readability.

**Recommendation:** use `|` block scalar for any summary containing newlines, plain double-quoted scalar for single-line summaries. Implement a helper that chooses the format.

### 3. Where exactly does summarization happen in the collect flow?

The plan says "after quality checks, before `write_new_document`". But `write_new_document` is called from both `collect_html` (HTML path) and directly in `collect_url` (YouTube path). The summary generation must be wired into both paths.

**Decision:** put the summary call inside `write_new_document` itself, not before it. That way both HTML and YouTube paths get summaries without duplicating the call.

### 4. `generate` orchestrator owns the tokio runtime — is that a problem?

Each `bo collect` call would spin up a new tokio runtime for the LLM call. This is the same pattern as compile. No issue for single-URL collects. If bo ever supports batch collection, this would need refactoring, but that's out of scope.

### 5. The prompt truncates body to 4000 words — is that enough context?

4000 words ≈ ~5000 tokens ≈ well within any model's context. The concern is the opposite: is 4000 words enough of the document to produce a good summary? For a 15,000-word article, you're summarizing only the first quarter.

**Decision:** acceptable for MVP. The first 4000 words of most articles contain the thesis, introduction, and key arguments. Edge case (conclusion-heavy docs) is a quality issue, not a correctness issue.

## Edge cases not covered by tasks

| Case | Expected behavior | Status |
|------|-------------------|--------|
| YouTube transcript collection | Should also get a summary | Not explicit — covered if summary is in `write_new_document` |
| Body is entirely markdown headers/links (no prose) | Fallback produces headers/links as "summary" | Acceptable but ugly |
| LLM returns empty string | Treat as failure, fall back to deterministic | Not specified |
| LLM returns >500 words | Store as-is (it's the model's output) | Acceptable |
| Summary contains YAML-breaking chars (`\n---\n`) | Must be properly escaped in frontmatter | Covered by format choice |
| `bo show` displaying the summary | Shows raw frontmatter field? | No change needed — `show` reads frontmatter as-is |
| `bo compile` reading leaves with summaries | Compile reads full content — summary is just another frontmatter field | No change needed |

## Dependencies

- **`OPENAI_API_KEY` env var** — same as compile. No new secrets or config.
- **Existing `engine::llm` module** — stable, no changes needed.
- **`tokio` crate** — already a dependency.
- **No new crates.**

The only external dependency is the OpenAI API availability for the LLM path. The fallback ensures collect never blocks on this.

## Recommendation

**Ready to implement.** Two things to nail down during implementation (not blocking):

1. **YAML format choice:** use `|` block scalar when summary contains newlines, double-quoted scalar otherwise. Implement as a formatting helper in `leaf.rs`.
2. **Wire summary into `write_new_document`** (not before it) so both HTML and YouTube paths get summaries without duplication.

No blockers. The feature is well-scoped, additive, and the fallback ensures zero regression risk.
