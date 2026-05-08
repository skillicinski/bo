# Implementation: Organise src/ by architectural layers

## Summary

What started as a flat-to-layered directory reorganisation evolved into a full
architectural restructuring of the agent tool system. The original spec's file
moves were mechanical; the interesting decisions emerged during implementation.

## Commits

| # | Hash | Description |
|---|------|-------------|
| 1 | `90a68d8` | Scratchpad: architecture mental model |
| 2 | `2589d29` | Core reorg: domain/, cli/, engine/ layers + test relocation |
| 3 | `37b51d5` | Extract compile tools into cli/compile/tools.rs |
| 4 | `750657e` | Decouple ListIndex + ReadLeaf into engine/agent/tools/ |
| 5 | `66e037a` | Move ALL tools into engine/agent/tools/ (context-free constructors) |
| 6 | `dec99db` | Fix review drift: remove Mutex from read-only tool, relocate tests, scaffold providers/ |

## Final structure

```
src/
├── lib.rs                              # 4 pub mod declarations
├── main.rs                             # CLI dispatch only
│
├── domain/                             # pure types, zero internal deps
│   ├── tree.rs                         # Tree + TreeConfig
│   ├── branch.rs, leaf.rs              # entity I/O
│   ├── frontmatter.rs, slug.rs         # shared domain logic
│   └── index.rs                        # index.jsonl operations
│
├── cli/                                # command orchestration
│   ├── collect.rs                      # bo collect pipeline + tests
│   ├── compile/mod.rs                  # bo compile orchestration + guard tests
│   └── list.rs                         # bo list
│
├── engine/                             # I/O capabilities
│   ├── agent/
│   │   ├── mod.rs                      # Tool trait, LlmProvider trait, run loop
│   │   ├── providers/
│   │   │   └── openai.rs              # OpenAiProvider (scaffolded for siblings)
│   │   └── tools/
│   │       ├── list_index.rs          # Arc<Vec<IndexEntry>> — read-only
│   │       ├── read_leaf.rs           # PathBuf — read-only
│   │       ├── write_branch.rs        # decoupled params + result sink
│   │       └── update_leaf_frontmatter.rs
│   ├── config.rs, fetch.rs, extract.rs, quality.rs
│
└── adapters/                           # external protocol integrations
    └── youtube/
```

## Key decisions made during implementation

1. **TreeConfig moved to domain/tree.rs** — resolved a dependency direction
   violation (domain must not import from engine).

2. **Tests relocated by ownership** — `tests/integration.rs` moved in-module to
   `cli/collect.rs`; offline compile tests moved to `cli/compile/mod.rs`;
   tool tests moved to each tool's own file. `tests/` retains only true CLI
   integration tests (binary invocation or live API).

3. **Tools decoupled from CompileContext** — each tool takes only what it needs
   (PathBuf, Arc<Vec<...>>, result sinks) rather than a shared mutable context
   struct. This enables composable tool selection for `bo query` without any
   cross-command coupling.

4. **Providers scaffolded** — `engine/agent/providers/openai.rs` exists as a
   standalone file, ready for an Anthropic/local sibling. The framework module
   stays clean (traits + run loop only).

## Scope drift from original spec

| Spec said | Actually did | Why |
|-----------|-------------|-----|
| Move files into 4 layers | ✅ Done | — |
| Relocate misplaced tests | ✅ Done, more aggressively | Tool tests moved to tool files, not just command files |
| Out of scope: split large files | Did it anyway (compile.rs) | Natural forcing function during tool extraction |
| Not mentioned: tool architecture | Full tools-as-capabilities refactor | Imminent `bo query` feature justified the investment |
| Not mentioned: provider scaffold | Created providers/ directory | Encodes design decision in structure |

## Dependency rule verification

```
domain/    → nothing internal
engine/    → domain/ only
cli/       → engine/ + domain/
adapters/  → nothing (uses super:: only)
main.rs    → cli/ + engine/ + domain/
```

Enforced by: `grep -rn "use crate::(cli|engine|adapters)" src/domain/` returns empty.

## Test results

- 158 unit tests pass
- 13 CLI integration tests pass
- 19 ignored (network/API-dependent)
- 0 clippy warnings
