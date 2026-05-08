# ADR-001: Deterministic Pipelines Over Internal Agent Loops

**Status:** Accepted
**Date:** 2026-05-08

---

## Context

Bo is a CLI tool that collects web content into a markdown tree (leaves) and uses an LLM to compile cross-cutting concept pages (branches). The initial implementation of `bo compile` used an internal agent loop: the LLM is given tools (`list_index`, `read_leaf`, `write_branch`, `update_leaf_frontmatter`) and iterates in a tool-calling loop until it decides it's done, with a step limit of 50.

The agent pattern is designed for workflows where the plan is unknown upfront — where the LLM must decide what to do next based on intermediate results. However, bo's compile workflow is entirely predetermined: list all docs, read all docs, identify patterns, write concept pages, update all frontmatter. The LLM's only creative contribution is pattern identification and synthesis — not workflow orchestration.

The agent loop has concrete costs:

- **Token waste:** each of ≤50 steps replays full message history
- **Fragility:** the agent can skip leaves, forget updates, or loop without progress
- **Step limit ceiling:** 50 steps caps the collection at ~40 leaves before truncation
- **Untestable:** concept extraction logic is entangled with tool dispatch
- **Cost:** N+2 API calls minimum vs 1–2

Additionally, bo's CLI commands are intended to serve as reliable tools for external agents (Claude Code, Cursor, custom agents via MCP). An internal agent adds unpredictability to what should be a deterministic primitive.

---

## Decision

All LLM-powered bo commands will follow the **deterministic pipeline with structured LLM call** pattern:

1. Code gathers context (read index, read leaves, select relevant content)
2. One structured-output LLM call performs the creative/analytical work
3. Code writes results (branches, frontmatter, answers, reports)

Specifically:

- **`bo compile`:** code reads all leaves → single LLM call produces JSON `{branches, leaf_assignments}` → code writes branch files and updates frontmatter
- **`bo query`:** code selects relevant context → single LLM call synthesizes answer with citations → code outputs/files result
- **`bo lint`:** code gathers summaries → single LLM call identifies issues → code formats report

The existing `LlmProvider` trait is retained for provider abstraction. The `Tool` trait and agent loop (`engine/agent/mod.rs`, `engine/agent/tools/`) will be removed once compile is migrated.

Bo does not contain orchestration logic. External agents compose bo's CLI commands (or MCP tools) to automate workflows. Bo is a Unix-philosophy primitive: do one thing reliably per invocation.

---

## Consequences

**Positive:**

- ~20x fewer API calls per compile, dramatically cheaper per-run
- No step-limit ceiling — collection size limited only by context window (solvable with chunking/parallelisation pattern)
- Fully testable — mock one LLM response, test all surrounding logic deterministically
- Predictable execution — external agents can rely on bo commands producing consistent results
- CLI output with `--json` flag makes bo composable by any agent/script

**Negative:**

- Single-call compile requires all leaf content to fit in one context window (mitigated by chunking for large collections)
- Loses flexibility for future commands that genuinely need open-ended exploration (mitigated: can reintroduce agent pattern for specific commands like `bo research` if needed)

---

## Alternatives Considered

1. **Keep the internal agent loop** — rejected because the workflow is predetermined and the agent adds cost and fragility without value.

2. **Multi-step prompt chain** (multiple LLM calls in fixed sequence) — viable for very large collections where content exceeds context window; may be adopted as a scaling strategy but not the default.

3. **RAG-based retrieval for query** — deferred; Karpathy's insight is that at small-to-medium scale (~100s of docs), maintained indices + direct context is sufficient without vector infrastructure.

---

## References

- Karpathy, *LLM Knowledge Bases* (April 2, 2026): raw → compile → wiki pattern, knowledge base as product gap
- Gulli, *Agentic Design Patterns*: prompt chaining (pattern #1) over agent loops when workflow graph is known upfront
- Current implementation: `src/engine/agent/` (~1100 LOC)
