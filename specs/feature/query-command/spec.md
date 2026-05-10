# Spec: `bo query` V1 — LLM-Synthesized Answers with Citations

## Problem statement

The core product loop (`collect → compile → query/navigate`) is incomplete. Users collect sources and compile branches, but have no way to ask questions over their knowledge base and receive synthesized answers. `bo search` provides deterministic retrieval but outputs raw file matches — users must still read and synthesize manually. The gap between "find relevant files" and "get an answer" is where the product value lives.

## User-facing requirements

1. **Basic usage**: `bo query <question>` accepts a natural-language question and returns a prose answer synthesized from the tree's content, with wikilink citations to source leaves (e.g. `[[leaf-slug]]`).

2. **Provider requirement**: the command requires a configured LLM provider (API key via environment variable or config). If no provider is available, the command fails with a clear error message directing the user to configure one. No silent degradation to search — that's what `bo search` is for.

3. **Model configuration**: uses a `query_model` field in `~/.bo/config.json`. Defaults to `gpt-4o` when absent. Independent of `compile_model`.

4. **Retrieval**: uses existing search infrastructure to find relevant leaves. Extracts keywords/terms from the natural-language question, runs search, and selects top results for context assembly.

5. **Context assembly**: assembles context from retrieved leaves for the synthesis call. Uses leaf summaries for initial selection breadth, full bodies of top-k for synthesis depth. Respects model context window limits.

6. **Synthesis output**: the answer is a prose response (1–3 paragraphs typical) with inline wikilink citations to the source leaves used. Citations reference leaf slugs that can be inspected via `bo show <slug>`.

7. **No results handling**: if search retrieval returns no relevant matches, the command reports "no relevant sources found in tree" and exits — does not ask the LLM to hallucinate from nothing.

8. **JSON output**: `--json` flag emits structured output:
   ```json
   {
     "answer": "prose answer text with [[citations]]...",
     "citations": [
       { "slug": "leaf-slug", "title": "Leaf Title", "file": "path/to/leaf.md" }
     ],
     "model": "gpt-4o",
     "leaves_consulted": 5
   }
   ```

9. **Read-only**: the command never modifies the tree, index, or any file.

10. **Exit codes**: 0 = answer produced, 1 = no relevant sources found, 2 = provider/config error.

## Success criteria

- A user with a 20+ leaf tree and `OPENAI_API_KEY` set can run `bo query "what is X?"` and receive a coherent answer citing actual leaves in the tree.
- Citations are valid — every `[[slug]]` in the answer corresponds to a real leaf verifiable via `bo show <slug>`.
- Without an API key, the command fails immediately with actionable error text (not a panic, not a cryptic HTTP error).
- `--json` output is valid JSON parseable by `jq` and conforms to ADR-002 schema guidelines.
- Answer quality is testable: for a known tree with known content, the answer addresses the question using information present in the tree (not hallucinated external knowledge).
- Latency is dominated by the LLM call — retrieval and context assembly add <200ms overhead.

## Out of scope

- Agentic tree navigation (V2 architecture per ADR-003) — V1 uses code-driven retrieval only.
- Cross-query context/session continuity — each invocation is stateless.
- Multiple provider support (Anthropic, OpenRouter, Gemini) — OpenAI first, others in follow-up.
- Streaming output — answer is returned complete.
- Follow-up/conversational mode — single question, single answer.
- Searching/citing branch (compiled) content — leaves only for V1.
- Custom system prompts or answer style configuration.

## Open questions

None — resolved during spec discussion. V2 evolution path documented in ADR-003.
