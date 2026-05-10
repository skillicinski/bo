# Analysis: `bo query` V1

## Risk assessment

### Medium risks

**1. Term extraction produces degenerate inputs**
Questions like `"what is it?"` yield zero terms after stop-word removal. Questions like `"how does it handle A and B?"` yield single-letter terms (`["a", "b"]`) that match everywhere. The plan has no floor — retrieval with empty or trivially short terms will either error confusingly or return noise.

*Mitigation:* Define minimum term length (≥2 chars). If extraction produces zero usable terms, exit with a specific error ("could not extract search terms — try rephrasing").

**2. OR-semantics retrieval returns noise**
Any single term matching once in a 5000-word leaf produces score > 0. A question about "Rust memory" will match every leaf that mentions "Rust" regardless of memory content. Top-10 will be polluted by low-relevance hits.

*Mitigation:* Acceptable for V1 — the LLM is responsible for deciding what's relevant from the assembled context. The breadth tier (summaries only) gives it awareness without consuming full-body budget on noise. Revisit if answer quality is poor in dogfood.

**3. Possessives create matching gaps**
The plan shows `"rust's"` as an extracted term, but leaves may contain `Rust's`, `Rusts`, or just `Rust`. The apostrophe creates a substring mismatch: `"rust's"` won't match `"rust "` via `.contains()`.

*Mitigation:* Strip possessive suffixes (`'s`, `'t`, `'re`, `'ve`, `'d`, `'ll`) during extraction. Treat `"rust's"` → `"rust"`.

**4. Async runtime pattern**
`main()` is sync. Compile/summary use `tokio::runtime::Builder::new_current_thread().block_on()` inline. Query will follow the same pattern. No risk, but worth noting the synthesis function should follow the established pattern exactly.

### Low risks

**5. Structured output model compatibility**
If user sets `query_model` to a model that doesn't support `response_format: json_schema`, the call fails. Low risk because the default (gpt-4o) supports it, and it's a user-initiated misconfiguration with a clear LLM error surfaced.

**6. Large single leaf exceeds budget alone**
A YouTube transcript or long article could be 20k+ words. If one leaf body consumes a third of the 60k budget, fewer leaves get depth. Low risk for V1 — the truncation logic handles this, and answer quality degrades gracefully.

## Gap analysis

### Must resolve before coding

**1. All-stop-words question**
`bo query "what is this?"` → zero terms after extraction. Plan doesn't specify exit behaviour. **Decision needed:** exit 1 ("no relevant sources") or exit 2 ("could not extract search terms")?

*Recommendation:* Exit 2 with "could not extract meaningful terms from question — try rephrasing with specific keywords." This is a usage error, not a retrieval failure.

**2. Citation stripping from prose**
Plan says "strip hallucinated citations" from `cited_slugs`. But the `answer` text still contains `[[invalid-slug]]` wikilinks. Do we also regex-replace them out of the prose?

*Recommendation:* Yes — strip `[[slug]]` from answer text for any slug not in the validated set. A dangling wikilink in output is confusing. Replace with just the text content or remove the brackets.

**3. Missing summary in frontmatter**
Older leaves (pre-summary-feature) won't have a `summary:` field. Context assembly assumes summaries exist.

*Recommendation:* Fall back to first ~200 words of body (same as `generate_fallback` logic) when summary field is absent. Don't fail.

### Underspecified but safe to decide during implementation

**4. Question argument handling**
Spec says `bo query <question>` — is this a single string arg (requires shell quoting: `bo query "what is X?"`) or multiple args joined (like search's `terms: Vec<String>`)? 

*Recommendation:* Single `String` arg with `#[arg(required = true, trailing_var_arg = true, num_args = 1..)]` collecting all remaining args into one joined string. This lets `bo query what is ownership` work without quotes.

**5. `leaves_consulted` semantics**
Data model says "number of leaves whose full body was sent" (depth tier). Confirm this is the depth count (≤5), not the total retrieval set (≤10).

**6. Slug derivation from file path**
Index stores `file: "leaves/understanding-ownership.md"`. Slug is `understanding-ownership` (strip directory prefix + `.md` suffix). This transformation isn't codified anywhere — define it explicitly in the query module.

## Edge cases

| Scenario | Expected behaviour | Covered by tasks? |
|----------|-------------------|------------------|
| Question is one word: `bo query ownership` | Valid — single term, retrieval works | Partially (no explicit test) |
| All stop words: `bo query "what is the thing?"` | Exit 2, actionable error | ❌ Not covered |
| Tree has 1 leaf | top-10 = 1, top-5 = 1, synthesis still runs | ❌ Not covered |
| Tree has 0 leaves (seeded, no collections) | Exit 1, "no sources collected yet" | ✅ In error table |
| Leaf body is empty (frontmatter only) | Score 0 unless title/summary matches, excluded naturally | ❌ Not explicit |
| Leaf frontmatter parse fails | Skip leaf, log warning, continue | ❌ Not specified |
| LLM returns empty `cited_slugs` array | Valid — answer without citations is permitted | ❌ Not explicit |
| LLM cites a slug not in retrieval set | Strip from cited_slugs AND from answer prose | ❌ Gap (see above) |
| Single leaf body > 60k words | Truncate that body to fill budget, other leaves get summaries only | ❌ Not explicit |
| Question is 500+ words (paste a paragraph) | Many extracted terms, diluted scores — still works, just noisier | ❌ Not explicit |
| `index.jsonl` doesn't exist | Treat as empty tree — exit 1 | ✅ Covered by index::read_index returning empty vec |

## Dependencies

| Dependency | Status | Risk |
|-----------|--------|------|
| `engine::llm::LlmProvider` trait + `OpenAiProvider` | Exists, used by compile + summary | None |
| `domain::index::read_index` | Exists | None |
| `domain::frontmatter::parse` | Exists, returns `(Mapping, String)` | None — but extracting summary from Mapping requires `mapping.get("summary")` YAML value access |
| `OPENAI_API_KEY` env var pattern | Established in compile + summary | None |
| `serde_json` / `serde` | Already dependencies | None |
| OpenAI API structured output support | Available on gpt-4o, gpt-4o-mini | None for default model |

No blocking dependencies. All integration points exist and are proven.

## Recommendation

**Ready to implement** with three decisions made upfront:

1. **All-stop-words → exit 2** with "could not extract meaningful terms" message. Add to task 2 (term extraction) as an explicit edge case + test.

2. **Invalid citations stripped from prose too** — regex-remove `[[slug]]` for any slug not in validated set. Add to task 3 (citation validation).

3. **Missing summary fallback** — use first 200 words of body. Add to task 2 (retrieval) when loading leaf metadata.

These are small additions to existing tasks, not new tasks. No architectural rework needed.

### Tasks amendment

Add to task 2 tests:
- All-stop-words input → error
- Single-word question → valid
- Minimum term length filter (drop terms < 2 chars)

Add to task 3:
- Strip invalid `[[slug]]` from answer prose, not just from `cited_slugs`
- Handle missing summary field in frontmatter (fallback to body truncation)

Add to integration test:
- Tree with 1 leaf (boundary case for top-k logic)
- Canned response with an invalid citation → verify it's removed from both prose and citations list
