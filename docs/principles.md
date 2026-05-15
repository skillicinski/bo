# Principles

Architectural commitments and strategic framing for bo. These evolve slowly — revisit when fundamentals shift, not per release.

## Storage philosophy

Markdown tree is user-owned and inspectable. Any DB, index, or snapshot layer is acceleration and safety — never the source of truth. The tree must be rebuildable from its own content.

## Architectural stance

Bo is a deterministic pipeline tool, not an autonomous agent. LLM-powered commands (compile, query, summary) follow a fixed pattern: code gathers context → one structured-output LLM call → code writes results.

Bo's CLI commands are reliable primitives. External agents (Claude Code, Cursor, custom setups) compose bo commands to automate knowledge base workflows. An MCP server is a natural extension of the CLI for agent consumption. See `adrs/001.md`.

Agent loops are permitted for specific commands (query V2 per ADR-003) under strict constraints: read-only, bounded steps, single-purpose. All other commands remain deterministic pipelines.

## Sequencing principles

1. **Deterministic inspection before LLM-dependent features** — users and agents must be able to see and search tree state without network or provider keys.
2. **Read-only retrieval before mutation** — query should not modify the tree; compile safety should gate writes.
3. **Lexical/deterministic search before vector/embedding search** — avoid DB infrastructure until the simple path is proven insufficient.
4. **One command per session** — promote and implement one right-sized feature at a time.
5. **Adapter breadth is lower priority than core loop completeness** — don't add new source adapters until inspect/query/compile safety exist.
6. **Bo is a tool, not an agent** — LLM calls are single-shot structured output within deterministic pipelines; orchestration belongs to the user or their agent.

## Product strategy

### Two surfaces, one engine

1. **CLI (now)** — discrete operations, user controls when things happen. Power users, agents, composability. Each release iterates on engine intelligence.
2. **Hosted (future)** — the engine running continuously, ingest-triggered everything. Managed experience. GUI for managing trees. Platform-to-platform integrations over HTTP.

### The CLI's role

- Proving ground for the engine. Compile gets smarter, quality gates improve, retrieval deepens — all validated through CLI dogfood and the test corpus.
- Power-user / developer interface. Full control, `--json` for agents, composable with external tools.
- Free with BYOK. Stays OSS and free forever.

### The hosted deployment's role

- The engine graduates to running unattended in a backend.
- Ingest-triggered compilation — collecting a source immediately integrates it into the tree.
- GUI for tree management, browsing, query.
- HTTP integrations: RSS watchers, browser extensions, Slack/Discord bots feeding sources, API for third-party tools.
- Hosted inference (Bedrock) with provisioned keys and spending caps.
- The managed LLM wiki experience for non-technical users.

### Engine architecture implication

The engine's operations must be cleanly separable from CLI invocation:
- Composable primitives (`write_leaf`, `read_leaf`, etc.) usable by both CLI commands and a backend orchestrator.
- The difference between CLI and hosted is *who calls the engine* and *what triggers operations* — not the operations themselves.
- CLI: human types command → engine runs → output.
- Hosted: event fires (new source, schedule, webhook) → orchestrator calls engine → result stored/notified.

### Differentiation

The product gap (per Karpathy's "LLM Wiki" gist, 2026): no standalone tool makes the persistent, compounding wiki pattern easy to operate. People orchestrate it ad-hoc with LLM agents + Obsidian.

Bo's path:
1. **v0.0.x**: Engine intelligence through CLI iteration (incremental compile, quality gates, retrieval depth).
2. **v0.1.x**: Provider flexibility + release polish. The CLI is good enough to show people.
3. **v0.x**: Agentic retrieval (V2), bloom/grow, auto mode switching. The engine becomes genuinely intelligent.
4. **v1.0+**: Hosted deployment. The engine runs continuously. Ingest-triggered compilation. GUI. Integrations.

## References

- ADR-001: Deterministic pipelines over internal agent
- ADR-003: Agentic retrieval for query V2
- Karpathy, "LLM Wiki" (2026): https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f
