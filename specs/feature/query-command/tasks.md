# Tasks: `bo query` V1

## Config

- [ ] Add `query_model: Option<String>` to `Config` struct in `src/engine/config.rs`
- [ ] Add `effective_query_model()` accessor defaulting to `"gpt-4o"`
- [ ] Verify existing config read/write tests pass, add test for new field round-trip

## Query module: term extraction + retrieval

- [ ] Create `src/cli/query.rs`, add `pub mod query` to `src/cli/mod.rs`
- [ ] Define `QueryError` enum covering all error conditions (no provider, no results, LLM failure, parse failure)
- [ ] Implement stop-word term extraction: strip question words/articles/prepositions, lowercase, strip boundary punctuation
- [ ] Implement OR-semantics leaf retrieval: read all leaves via `index.jsonl`, parse frontmatter (title, url, summary), score by term density, filter score > 0, sort descending, return top-10
- [ ] Unit tests: term extraction (various question shapes, empty input, all-stop-words input)
- [ ] Unit tests: retrieval scoring (fixture leaf content, verify OR semantics and ranking order)

## Context assembly + synthesis

- [ ] Implement two-tier context assembly: top-10 get slug/title/url/summary, top-5 get full body; cap at ~60k words total
- [ ] Define synthesis system prompt (answer from sources only, cite with `[[slug]]`, concise)
- [ ] Define structured-output schema for `SynthesisResponse { answer, cited_slugs }`
- [ ] Implement synthesis call via `LlmProvider` (construct messages, call, parse response)
- [ ] Implement citation validation: verify cited slugs exist in retrieval set, strip hallucinated citations
- [ ] Unit tests: context assembly respects token budget, truncates when over limit
- [ ] Unit tests: citation validation strips invalid slugs, preserves valid ones

## Output formatting + CLI wiring

- [ ] Implement human-mode renderer (prose answer + Sources list with wikilinks)
- [ ] Implement JSON-mode renderer (ADR-002 compliant schema: answer, citations, model, leaves_consulted)
- [ ] Add `Query` variant to `Commands` enum in `src/main.rs` with `question: String` arg
- [ ] Wire dispatch: read config → check API key → call query pipeline → render output
- [ ] Handle exit codes: 0 = success, 1 = no results, 2 = provider/config error
- [ ] Verify: `bo query --help` shows correct usage, `bo query "test"` without API key exits 2 with actionable message

## Integration test

- [ ] Create end-to-end test with temp tree directory containing 5+ fixture leaves with frontmatter and summaries
- [ ] Mock `LlmProvider` returning a canned `SynthesisResponse` with valid and one invalid citation
- [ ] Assert full pipeline: terms extracted → correct leaves retrieved → context assembled → mock called with expected messages → output contains answer with valid citations only → invalid citation stripped
- [ ] Assert JSON output is valid and schema-conformant
