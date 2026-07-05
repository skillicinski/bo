# Code architecture — how code in this project is structured

Companion doc: `docs/architecture.md` — the *decisions* behind these rules.

## Engine vs CLI layering

The load-bearing line:

- **Engine (`src/engine/*`)** — reusable primitives shared across commands. Holds `fetch`, `extract`, `llm`, `pending`, `retrieval`, `quality`, `summary`. No command-specific knowledge.
- **CLI (`src/cli/<cmd>/*`)** — that command's orchestration, including its pipeline stages.

The discipline cuts both ways:

1. Don't push command-specific logic into the engine. Example: "a branch is stale when its leaves are deleted and must be repaired" is compile-domain knowledge, so it lives in `cli/compile/`, not `engine/`. `Branch` is data in `domain`; the staleness *rule* belongs to the command that enforces it.
2. Don't carve the engine up per-command. The engine stays shared primitives; it doesn't gain per-command sub-folders.

## Pipeline-stage modularity

When a command is a multi-stage transformation with a **non-trivial intermediate contract** between stages, split it into one module per stage — each owning its stage's types.

- **Compile qualifies** — raw LLM response → validated plan → manifest delta → committed tree. Each transition is a contract, so `cli/compile/` splits into `plan`, `prompt`, `parse`, `execute`, `schema`, `validation`, `repair`, `render`.
- **Query qualifies** — question → retrieved docs → assembled context → synthesized answer → validated citations.
- **Collect qualifies** — URL → fetched → extracted → quality-checked → summarized → written.
- **Status does NOT** — "read state, format it" is one transformation. `status.rs` as a single module is correct; a folder would be over-engineering.

## Type-system enforcement of stage contracts

A validated value is a **different type** than an unvalidated one. This is the "make illegal states unrepresentable" principle applied to a pipeline. A `CompilePlan` (validated) cannot be constructed from a raw LLM response without going through validation — the type system enforces it. A function that expects a validated plan takes `CompilePlan`, not `RawBranch`. The module that owns each stage owns that stage's type — `validation` defines what "valid" means and exports the validated type.

## Internal visibility (`pub(super)`)

Modules within a command use `pub(super)` for functions that are internal to that command but shared across its stages. A function like `leaf_resolver` or `build_full_delta` is reachable within `cli/compile/` but invisible to `query` or `collect`. This is the visibility discipline that keeps command internals from leaking into each other — no global `pub` for command-internal plumbing.

## Transfer test

Before adding a module or a folder: "does this command have ≥2 stages with a contract between them that could be violated?" Yes → split into modules per stage. No → one file.
