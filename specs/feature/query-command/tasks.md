# Tasks: `bo query` V1

## Config

- [x] Add `query_model: Option<String>` to `Config` struct in `src/engine/config.rs`
- [x] Add `effective_query_model()` accessor defaulting to `"gpt-4o"`
- [x] Verify existing config read/write tests pass, add test for new field round-trip

## Query module: term extraction + retrieval

- [x] Create `src/cli/query.rs`, add `pub mod query` to `src/cli/mod.rs`
- [x] Define `QueryError` enum covering all error conditions (no provider, no results, LLM failure, parse failure)
- [x] Implement stop-word term extraction: strip question words/articles/prepositions, lowercase, strip boundary punctuation, strip possessive suffixes (`'s`, `'t` etc.), drop terms < 2 chars
- [x] Handle degenerate extraction: if zero terms remain after filtering, return `QueryError` (exit 2, "could not extract meaningful terms from question")
- [x] Implement OR-semantics leaf retrieval: read all leaves via `index.jsonl`, parse frontmatter (title, url, summary), score by term density, filter score > 0, sort descending, return top-10. Fall back to first 200 words of body when summary field is absent in frontmatter.
- [x] Unit tests: term extraction (various question shapes, single word, all-stop-words → error, possessives stripped, short terms dropped)
- [x] Unit tests: retrieval scoring (fixture leaf content, verify OR semantics and ranking order, leaf with missing summary uses body fallback)

## Context assembly + synthesis

- [x] Implement two-tier context assembly: top-10 get slug/title/url/summary, top-5 get full body; cap at ~60k words total
- [x] Define synthesis system prompt (answer from sources only, cite with `[[slug]]`, concise)
- [x] Define structured-output schema for `SynthesisResponse { answer, cited_slugs }`
- [x] Implement synthesis call via `LlmProvider` (construct messages, call, parse response)
- [x] Implement citation validation: verify cited slugs exist in retrieval set, strip hallucinated citations from `cited_slugs` AND regex-remove invalid `[[slug]]` wikilinks from answer prose
- [x] Unit tests: context assembly respects token budget, truncates when over limit
- [x] Unit tests: citation validation strips invalid slugs from list and from prose, preserves valid ones

## Output formatting + CLI wiring

- [x] Implement human-mode renderer (prose answer + Sources list with wikilinks)
- [x] Implement JSON-mode renderer (ADR-002 compliant schema: answer, citations, model, leaves_consulted)
- [x] Add `Query` variant to `Commands` enum in `src/main.rs` with `question: String` arg
- [x] Wire dispatch: read config → check API key → call query pipeline → render output
- [x] Handle exit codes: 0 = success, 1 = no results, 2 = provider/config error
- [x] Verify: `bo query --help` shows correct usage, `bo query "test"` without API key exits 2 with actionable message

## Integration test

- [x] Create end-to-end test with temp tree directory containing 5+ fixture leaves with frontmatter and summaries (include one leaf without summary field)
- [x] Mock `LlmProvider` returning a canned `SynthesisResponse` with valid and one invalid citation
- [x] Assert full pipeline: terms extracted → correct leaves retrieved → context assembled → mock called with expected messages → output contains answer with valid citations only → invalid citation stripped from both prose and list
- [x] Assert JSON output is valid and schema-conformant
- [x] Boundary case: tree with 1 leaf — verify top-k logic handles gracefully
