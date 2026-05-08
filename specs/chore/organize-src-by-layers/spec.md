# Spec: Organise src/ by architectural layers

## Problem statement

The `src/` directory contains 16 flat module files alongside a single nested
`adapters/` directory. As the codebase grows, the flat layout obscures the
dependency relationships between modules. The existing `lib.rs` documents a
clear layered architecture that should be made explicit in the filesystem.

## Requirements

### 1. Reorganise src/ into layered subdirectories

Reorganise `src/` into subdirectories that mirror the established dependency
layers:

```
src/
├── main.rs              # CLI arg parsing + dispatch (unchanged location)
├── lib.rs               # crate root, declares submodules
│
├── domain/              # pure types, invariants, zero I/O
│   ├── mod.rs
│   ├── tree.rs
│   ├── branch.rs
│   ├── leaf.rs
│   ├── frontmatter.rs
│   ├── slug.rs
│   └── index.rs
│
├── cli/                 # subcommand orchestration pipelines
│   ├── mod.rs
│   ├── collect.rs
│   ├── compile.rs
│   └── list.rs
│
├── engine/              # shared capabilities with I/O
│   ├── mod.rs
│   ├── fetch.rs
│   ├── extract.rs
│   ├── quality.rs       # collection quality gate (fetch → extract → quality)
│   ├── agent.rs
│   └── config.rs
│
└── adapters/            # external protocol integrations (already exists)
    └── youtube/
```

Dependency rule: arrows point strictly downward.

```
main.rs → cli/ → engine/ → adapters/
                    ↓
               domain/ (used by all layers above, depends on nothing)
```

### 2. Relocate misplaced tests to their correct homes

Tests that call library internals directly (not through the CLI binary) are
unit/module tests and belong in `#[cfg(test)]` within their source module:

- `tests/integration.rs` (pipeline tests using `collect_html`) → move into
  `src/cli/collect.rs` as `#[cfg(test)]`
- `tests/integration_compile.rs` offline unit tests (3 tests) → move into
  `src/cli/compile.rs` as `#[cfg(test)]`

Tests that invoke the binary via `Command` are true integration tests and stay
in `tests/`:

- `tests/integration_cli.rs` — update fixture helper paths only
- `tests/integration_compile.rs` live API tests (5 tests, `#[ignore]`) — stay,
  update `bo::` paths to new module hierarchy
- `tests/integration_network.rs` — no changes (zero library imports)

## Success criteria

1. All existing tests pass (`cargo test`) with no changes to test logic.
2. `cargo clippy` and `cargo build` produce no new warnings.
3. Public API paths update to reflect the new module hierarchy
   (e.g. `bo::domain::leaf`, `bo::cli::collect`, `bo::engine::fetch`).
4. `lib.rs` re-exports only what `main.rs` needs — no crate-root re-exports of
   individual types.
5. The dependency direction rule holds: no `domain/` file imports from `cli/`,
   `engine/`, or `adapters/`.
6. No test file in `tests/` directly calls library functions that are
   exercising a single module's logic (those live in-module).

## Out of scope

- Splitting large files (compile.rs, list.rs) into sub-modules.
- Introducing new traits or abstractions.
- Changing any runtime behaviour.
- Modifying test assertions or coverage.
- Converting remaining `tests/` integration tests to use `assert_cmd`
  (future chore).

## Open questions

None.
