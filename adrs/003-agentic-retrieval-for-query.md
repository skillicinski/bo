# ADR-003: Agentic Retrieval for Query (V2 Architecture)

**Status:** Proposed
**Date:** 2026-05-10

---

## Context

ADR-001 establishes that bo commands follow deterministic pipelines: code gathers context → one LLM call → code writes results. This pattern is correct for commands where the workflow graph is known upfront (compile, lint, summary generation).

However, `bo query` has a fundamentally different retrieval problem. Unlike compile (which processes ALL leaves), query must FIND the right leaves — and the relevance decision is itself a creative/analytical task that benefits from LLM reasoning. A user asking "what are the tradeoffs of Rust's ownership model?" needs the system to:

1. Identify which branches are topically relevant
2. Within those branches, identify which leaves likely contain the answer
3. Peek at leaf metadata/summaries to confirm relevance
4. Potentially backtrack and try a different branch if initial selection was poor
5. Assemble final context from confirmed-relevant leaves
6. Synthesize an answer with citations

Steps 1–5 are open-ended exploration — the system doesn't know upfront which path through the tree will yield good results. This is precisely the case where an agent loop (tool-use pattern) outperforms a fixed pipeline, because the LLM's pattern recognition over structured documents exceeds what code-driven heuristics can achieve for relevance.

---

## Decision

`bo query` will evolve through two architectural phases:

### V1: Prompt-chaining pipeline (ADR-001 compliant)

Ships first. Code performs retrieval using existing search infrastructure, assembles context, and makes a single synthesis LLM call. Deterministic, testable, and sufficient for small-to-medium trees where search ranking finds the right content.

```
code: search(question) → top-k leaves
code: assemble context (summaries + full bodies of top-k)
LLM:  single structured-output call → synthesized answer with citations
code: format and output
```

### V2: Agentic tree navigation

Replaces V1's retrieval step with an LLM-driven exploration loop. The model receives tools to navigate the tree structure and decides its own retrieval path:

**Tools provided to the query agent:**

- `list_branches()` — see available compiled topic branches
- `read_branch(slug)` — read a branch page (contains leaf references)
- `peek_leaf(slug)` — read leaf frontmatter + summary (cheap)
- `read_leaf(slug)` — read full leaf content (expensive, token-budget-aware)
- `answer(text, citations)` — terminal tool, delivers the final synthesized answer

**Behavioural design:**

- The agent can backtrack: if a branch doesn't contain useful material, it can try another
- Token budget awareness: the harness tracks context usage and can signal when to stop gathering and start synthesizing
- Graceful termination: if the agent cannot find relevant material after exhausting reasonable paths, it reports "no relevant sources found" via the answer tool
- Step limit: hard cap prevents runaway loops (configurable, default ~15 steps)
- No mutation: query tools are strictly read-only

**Relevant agentic patterns (Gulli):**

- **Tool Use (#5):** core mechanic — LLM decides which tree navigation tools to invoke
- **Planning (#6):** model implicitly plans a retrieval strategy before executing
- **Reflection (#4):** backtracking when initial selection is poor is a form of self-correction
- **Resource-Aware Optimization (#16):** peek (summary) before read (full body) minimizes token spend

### V1 → V2 boundary

V1's interface (`bo query <question>` → answer with citations) remains identical in V2. The change is internal: how retrieval happens. V1 is the MVP; V2 is the target architecture once the tree is large enough that code-driven search ranking becomes insufficient.

The trigger for V2 implementation is when dogfood shows search-based retrieval consistently missing relevant content that a human would find by browsing the tree structure.

---

## Relationship to ADR-001

ADR-001 states: "can reintroduce agent pattern for specific commands like `bo research` if needed." This ADR exercises that exception for `bo query` V2 specifically, with the following constraints:

- The agent loop is **read-only** (no tree mutation)
- The agent loop is **bounded** (step limit, token budget)
- The agent loop is **single-purpose** (retrieval and synthesis only)
- All other bo commands remain deterministic pipelines

ADR-001's core thesis — that bo is a reliable primitive for external agents to compose — is preserved. Query's internal agent loop is an implementation detail; its external interface (`question in → answer out`) remains deterministic from the caller's perspective.

---

## Consequences

**Positive:**

- Query leverages LLM pattern recognition for relevance (the hard part of retrieval)
- Tree structure (branches → leaves) becomes a navigation aid, not just compile output
- Backtracking enables recovery from bad initial relevance guesses
- Peek-before-read minimizes token waste on irrelevant content
- Scales to large trees where flat search ranking degrades

**Negative:**

- Agent loop reintroduces non-determinism for query specifically (mitigated: bounded steps, read-only)
- Higher per-query cost than single-call synthesis (mitigated: step limit, peek-first strategy)
- More complex testing (mitigated: mock tool responses, test harness logic deterministically)
- V1 → V2 migration requires rework of retrieval internals (mitigated: identical external interface)

---

## Alternatives Considered

1. **Single-call retrieval forever** — rejected because relevance over structured trees is an open-ended problem that fixed pipelines solve poorly at scale.

2. **RAG with vector embeddings** — deferred; adds infrastructure (embedding model, vector store) that the tree's existing structure may render unnecessary. Revisit if agentic navigation proves insufficient.

3. **Multi-step prompt chain (fixed sequence)** — partially adopted as V1. Insufficient long-term because the optimal retrieval path varies per query and cannot be hardcoded.

---

## Open Direction: Cross-Query Context

V2's agentic retrieval produces structured intermediate state (branches explored, leaves read, relevance assessments) that could benefit subsequent queries. A follow-up question ("what about lifetimes specifically?") after an ownership query shouldn't re-discover the same branches from scratch.

**Sketch:**

- After each query, the agent's retrieval trace (explored paths, selected leaves, synthesized answer) is persisted as a lightweight session record
- On the next query, the agent receives the prior record and decides whether to reuse context or start fresh
- The relatedness decision belongs to the agent — no heuristic threshold in code
- A local DB (sqlite or similar) is the natural backend for session records, avoiding file proliferation in the markdown tree

**Not decided:**

- Session boundary semantics (time-based? explicit `--new` flag? agent-determined?)
- Storage format and location (`~/.bo/sessions/` vs sqlite vs tree-adjacent)
- Whether V1 should capture any trace data opportunistically for future V2 consumption
- Privacy implications of persisting query content locally

This is deferred until V2 implementation. Noted here so the retrieval tool design doesn't accidentally preclude session continuity.

---

## References

- ADR-001: deterministic pipelines, agent-loop exception clause
- Gulli, *Agentic Design Patterns*: Tool Use (#5), Planning (#6), Reflection (#4), Resource-Aware Optimization (#16), Memory Management (#8)
- Karpathy, *LLM Knowledge Bases*: navigable structure as retrieval advantage over flat document stores
