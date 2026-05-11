# ADR-003: Agentic Retrieval for Query (V2 Architecture)

**Status:** Proposed
**Date:** 2026-05-10
**Revised:** 2026-05-11

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

### The pattern: Agentic RAG over a structural index

The V2 architecture is best understood as **Agentic RAG** (Gulli, Ch. 14) — but with a critical difference from the standard embed-chunk-retrieve pipeline: bo's retrieval substrate is the **tree's branch/leaf hierarchy**, not a vector database.

In standard RAG, retrieval means: embed query → vector similarity search → top-k chunks. The intelligence is in the embedding space.

In bo's Agentic RAG, retrieval means: the LLM navigates a human-readable structural index (branches as topic clusters, leaves as documents) the way a researcher browses a wiki — by topic, then by document, with the ability to backtrack and try a different path. The intelligence is in the navigation decisions.

This is an architectural differentiator:
- No embedding model or vector store infrastructure required
- The retrieval index (branches + leaf summaries) is the same artifact users see when they run `bo list` or `bo show` — fully inspectable
- Tree structure produced by `compile` doubles as both a knowledge graph for humans and a navigation substrate for the query agent
- The tree is rebuildable from its own content (storage philosophy) — the retrieval index is never a black-box embedding table

Gulli's key question for when Planning applies: "does the *how* need to be discovered, or is it already known?" For query retrieval over a growing tree, the how (which branches, which leaves, in what order) varies per question and cannot be hardcoded. This is the fundamental argument for agentic retrieval over a fixed pipeline.

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

### V2: Agentic RAG over structural index

Replaces V1's retrieval step with an LLM-driven exploration loop. The model receives tools to navigate the tree structure and decides its own retrieval path. This is **Agentic RAG** — the agent actively interrogates relevance, decomposes multi-faceted questions into sub-queries across branches, and identifies knowledge gaps — but the retrieval substrate is bo's compiled tree hierarchy rather than a vector store.

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

**Composed agentic patterns (Gulli):**

V2 is not a single pattern — it composes several:

- **Agentic RAG (#14):** the governing pattern. The agent reasons about what to retrieve, validates source relevance, decomposes multi-faceted questions into sub-queries across branches, and identifies when the tree lacks coverage. The difference from standard Agentic RAG is the retrieval substrate: structural hierarchy instead of embedding space.
- **Planning (#6):** the retrieval path varies per query and must be discovered. The agent implicitly forms a strategy ("check the Rust branch first, then concurrency") before executing. Gulli: "does the how need to be discovered?" — yes.
- **Resource-Aware Optimization (#16):** peek (summary) before read (full body) minimizes token spend. Simple factual queries may resolve from summaries alone; complex synthesis requires full reads of fewer, confirmed-relevant leaves.
- **Reflection (#4):** backtracking when initial branch selection is poor. "This branch on inference scalability doesn't answer the ownership question — try another." Self-correction within the retrieval loop.
- **Tool Use (#5):** the execution mechanic — LLM decides which navigation tools to invoke and in what order.

Tool Use is the mechanic; Agentic RAG is the pattern. The distinction matters because it clarifies *why* the agent has tools (to navigate and reason about retrieval) rather than treating tool-calling as the design goal itself.

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

2. **Standard RAG with vector embeddings** — deferred. Adds infrastructure (embedding model, vector store) that bo's existing tree structure may render unnecessary. The tree's branch/leaf hierarchy already provides a navigable index — one that is human-readable, inspectable, and rebuildable from content. Vector search may complement structural navigation later but is not the primary retrieval mechanism. Revisit if agentic navigation over structure proves insufficient at scale.

3. **Multi-step prompt chain (fixed sequence)** — partially adopted as V1. Insufficient long-term because the optimal retrieval path varies per query and cannot be hardcoded.

4. **Pure tool-use framing** — considered but reframed. Tool Use (#5) is the execution mechanic, not the design pattern. Framing V2 as "give the LLM tools" misses the why: the agent needs tools in order to perform Agentic RAG over a structural index. The pattern is retrieval reasoning; tool-calling is how it acts on those decisions.

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

## V2 Trigger

V2 implementation begins when dogfood shows search-based retrieval consistently missing relevant content that a human would find by browsing the tree structure. Specific signals:

- Multi-hop questions that require material from multiple branches ("how does X compare to Y across these articles?")
- Queries where relevant leaves exist but V1's lexical search doesn't surface them due to vocabulary mismatch
- Trees large enough (hundreds of leaves, many branches) that flat top-k ranking produces noise from term overlap

The zero-citation patch (v0.0.1) addresses V1's answerability detection gap — the model hallucinating from parametric knowledge when retrieval finds irrelevant leaves. That is a different failure mode from retrieval quality and does not itself trigger V2.

---

## References

- ADR-001: deterministic pipelines, agent-loop exception clause
- Gulli, *Agentic Design Patterns*: Agentic RAG (#14), Planning (#6), Resource-Aware Optimization (#16), Reflection (#4), Tool Use (#5)
- Karpathy, *LLM Knowledge Bases*: navigable structure as retrieval advantage over flat document stores
