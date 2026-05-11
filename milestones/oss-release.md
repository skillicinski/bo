# OSS Release Milestone

North-star vision for the path from dogfood alpha to public open-source release. This document captures strategic framing, readiness gates, and sequencing constraints. It is not a living backlog — see `scratchpad/features.md` for right-sized implementation candidates.

## Target loop

```text
collect → compile → query/read/navigate → refine tree → repeat
```

Current state is closer to `collect → compile → files exist`.

## Storage philosophy

Markdown tree is user-owned and inspectable. Any DB, index, or snapshot layer is acceleration and safety — never the source of truth. The tree must be rebuildable from its own content.

## Architectural stance

Bo is a deterministic pipeline tool, not an autonomous agent. LLM-powered commands (compile, query, lint) follow a fixed pattern: code gathers context → one structured-output LLM call → code writes results. Bo does not contain orchestration or agent-loop logic.

Bo's CLI commands are reliable primitives. External agents (Claude Code, Cursor, custom setups) compose bo commands to automate knowledge base workflows. An MCP server is a natural extension of the CLI for agent consumption. See `adrs/001-deterministic-pipelines-over-internal-agent.md`.

## Readiness levels

**Experimental OSS** (~1–2 sessions):
- basic README expansion
- explicit experimental caveat
- at least one inspect command
- basic scan/survey or rebuild-index
- CI basics (fmt + clippy + test)
- LICENSE file in repo root (MIT, matching Cargo.toml)
- Cargo.toml metadata: description, repository, homepage, keywords
- zero-citation = not-answered patch (prevent hallucination on irrelevant retrieval)
- `bo config set` MVP (compile_model, query_model) — removes manual JSON editing friction

**Useful alpha** (~3–5 sessions):
- `bo query` MVP
- deterministic local search
- tree health scan/survey
- compile dry-run or snapshot
- better docs/examples

**Public beta** (~6–10 sessions):
- snapshots/history
- query with citations
- robust compile recovery
- dogfood expectations
- release packaging
- source-adapter hardening

## Sequencing principles

1. Deterministic inspection before LLM-dependent features — users and agents must be able to see and search tree state without network or provider keys.
2. Read-only retrieval before mutation — query should not modify the tree; compile safety should gate writes.
3. Lexical/deterministic search before vector/embedding search — avoid DB infrastructure until the simple path is proven insufficient.
4. One command per session — promote and implement one right-sized feature at a time from `scratchpad/features.md`.
5. Adapter breadth is lower priority than core loop completeness — don't add new source adapters until inspect/query/compile safety exist.
6. Bo is a tool, not an agent — LLM calls are single-shot structured output within deterministic pipelines; orchestration belongs to the user or their agent.

## Implementation candidates

See `scratchpad/features.md` for the authoritative backlog of right-sized feature candidates.
