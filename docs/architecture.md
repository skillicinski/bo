# Architecture

> **Canonical status:** this is the sole source of truth for bo's architecture. Other comments, agent instructions, and tests may link to or enforce this document; they must not define a competing layer model. Change this document and its enforcement in the same commit when the policy changes.

bo favors code that a human can trace top-to-bottom and a machine can constrain with types. It uses a small inward dependency order rather than a named architecture framework.

## Current topology

bo is one Cargo package with separate library and binary crates. The current production code has these direct top-level dependencies:

```text
main ───────→ cli
  └────────→ engine

cli ───────→ adapters
 ├─────────→ engine
 └─────────→ domain

engine ────→ domain
adapters ──→ no bo layer
domain ────→ no bo layer
```

This is an inventory, not the policy. In particular, direct dependencies may skip layers: the CLI uses domain types directly, and `main.rs` composes engine capabilities directly.

The layers are not a pure-I/O sandwich. Command-specific filesystem work exists in `cli`; reusable filesystem and network capabilities exist in `engine`; source-specific network translation exists in `adapters`. The boundary is ownership and reuse, not whether a function performs I/O.

## Target dependency policy

The canonical inward order is:

```text
main → cli → adapters → engine → domain
```

This is an ordering, not a required call chain. A layer may depend on any layer to its right, including skipping intermediate layers. It must not depend on a layer to its left.

| Layer | May depend on |
|---|---|
| `main` | `cli`, `adapters`, `engine`, `domain` |
| `cli` | `adapters`, `engine`, `domain` |
| `adapters` | `engine`, `domain` |
| `engine` | `domain` |
| `domain` | no other bo layer |

Dependencies within one layer are allowed, but cycles should be removed when they obscure ownership. Cross-layer paths in the library use explicit `crate::<layer>` paths; grouped crate-root imports and relative paths that can climb into another top-level layer are avoided so the architecture test can inspect every reference.

### Layer ownership

- **`main.rs` — process shell and composition root.** Owns argument parsing, process-wide tracing, stdout/stderr, exit codes, and construction of command dependencies. Keep domain policy out of it.
- **`cli` — command application layer.** Owns command-specific orchestration, policy, intermediate stage contracts, and human/JSON rendering. "A stale branch must be repaired during synthesis" belongs here. Command-specific I/O may remain here.
- **`adapters` — source-specific integrations.** Translates external protocols that do not belong to a generic engine capability. The current top-level adapter is YouTube ingestion, selected by `cli::collect`. It has no bo-layer dependencies today.
- **`engine` — reusable capabilities.** Owns command-neutral fetching, extraction, persistence, retrieval, LLM transport, retry policy, and other shared operations. A function belongs here only when its name and signature contain no command-specific vocabulary. Engine never imports CLI or top-level adapters.
- **`domain` — side-effect-free model and format contracts.** Owns entities, validated values, topology rules, serialization shapes, path naming, and document formatting. It performs no filesystem, network, or process I/O and imports no outer bo layer.

Provider implementations under `engine::llm::providers` are part of the LLM capability: they implement the engine-owned `LlmProvider` contract. Top-level `adapters` are instead ingestion-source integrations. Do not move either merely to make all external HTTP code share one directory.

Use traits only at a real interchangeable or testable boundary. Do not add ports, repositories, or per-layer crates solely to make the diagram look purer.

## Process I/O and diagnostics

`main` owns process streams. CLI renderers may produce human or JSON output, and interactive CLI code may prompt or diagnose. Engine, adapters, and domain return values, warnings, or errors instead of printing directly; this keeps reusable capabilities composable by another front end.

Known departures are recorded rather than hidden:

- `engine::fetch` and `engine::summary` currently write fallback/retry diagnostics to stderr. Return or trace those diagnostics when those paths are next changed.

These are bounded cleanup targets, not reasons for a broad layer rewrite.

## Public surfaces and visibility

The supported product interface is the executable: command behavior, machine-readable JSON envelopes, and documented on-disk formats.

Cargo compiles `lib.rs`, `main.rs`, and integration tests as separate crates. Consequently, `main` and tests can only reach public library items, and `src/lib.rs` currently exposes `adapters`, `cli`, `domain`, and `engine`. `cli` is therefore objectively public Rust visibility; it must not be described as private.

Reusable library code should live in `domain` and `engine`, but bo does not yet promise a stable Rust library API distinct from the CLI product. If real external Rust consumers require one, introduce a deliberate facade and move binary-only implementation behind it. Until then, do not add crate splits or re-export scaffolding for a hypothetical consumer.

Within a layer, use the narrowest practical visibility. Synthesis stages use `pub(super)` for command-internal contracts; engine internals use `pub(crate)` when the binary does not need them.

## State and format decisions

### Tree state is the topology source of truth

`{tree}/.bo/state.json` is the only topology record. Branches store leaf slugs; `TreeState::branches_for_leaf` computes the inverse in memory. `pending.json` is transaction recovery state and `journal.jsonl` is an operational log, not a second topology model.

### The domain vocabulary is typed

`Tree`, `Branch`, and `Leaf` are the serialized entities used by the rest of the system; there is no parallel record/entity hierarchy. Values with invariants use validated types such as `Slug`, `Title`, `Timestamp`, and `Url`. Domain modules format serialized content but do not read or write it; persistence belongs to the owning engine or CLI operation.

On-disk format behavior is guarded by round-trip or byte-level tests. Do not add abstraction solely to deduplicate coincident fields or serialization code.

## LLM trust and tool boundaries

Every structured LLM response is deserialized and validated against known domain state before mutation. A single validation failure rejects the whole change; no partial write is allowed.

The tool-calling split is intentional:

- `engine::llm` owns provider-neutral transport messages, tool-call protocol types, provider serialization, timeout, and retry behavior.
- `engine::agent` owns the bounded provider-neutral turn loop and generic tool contract.
- `cli::synthesize::agent` owns synthesis-specific tools and orchestration.

Tool arguments are untrusted input and become typed, validated values at the tool boundary.

## Command pipelines

Split a command into stage modules when an intermediate contract is independently meaningful and the split makes the workflow easier to trace. Synthesis is the exemplar: planning, prompting, parsing, validation, execution, repair, and rendering have distinct contracts.

Stage count alone is not a rule. A command may stay in one file while its stages remain clear; split it when a contract needs isolated ownership or work in the file has become difficult to trace. Do not scaffold folders for expected future complexity.

## Enforcement and escalation

`tests/architecture.rs` is the executable backstop for the dependency order. It scans every Rust file under `src/domain`, `src/engine`, `src/adapters`, and `src/cli`, not only `use` declarations, and enforces an unambiguous cross-layer path style so fully qualified calls are covered and common relative or aliased paths cannot bypass the check. Compiler visibility (`pub(super)`, `pub(crate)`) remains the first choice for narrower boundaries.

The source scan is a guardrail, not semantic proof. Separate crates are the hard Rust boundary, but a workspace split is not justified for one product and one implementation team. Reconsider it only when a layer has an independent consumer/release lifecycle or repeated real violations show that the lightweight check is insufficient.
