# Tasks: Organise src/ by architectural layers

## Scaffold

- [x] Create `src/domain/`, `src/cli/`, `src/engine/` directories
- [x] Create `src/domain/mod.rs` declaring: tree, branch, leaf, frontmatter, slug, index
- [x] Create `src/cli/mod.rs` declaring: collect, compile, list
- [x] Create `src/engine/mod.rs` declaring: fetch, extract, quality, agent, config
- [x] Rewrite `src/lib.rs` to declare four top-level modules: adapters, cli, domain, engine

## Domain layer

- [x] Move tree.rs, branch.rs, leaf.rs, frontmatter.rs, slug.rs, index.rs into `src/domain/`
- [x] Extract `TreeConfig` struct from config.rs into domain/tree.rs (remove `use crate::config::TreeConfig`)
- [x] Fix domain/branch.rs import: `crate::frontmatter` → `crate::domain::frontmatter`
- [x] Fix domain/leaf.rs import: `crate::frontmatter` → `crate::domain::frontmatter`

## Engine layer

- [x] Move fetch.rs, extract.rs, quality.rs, agent.rs, config.rs into `src/engine/`
- [x] In engine/config.rs: remove `TreeConfig` definition, add `use crate::domain::tree::TreeConfig`
- [x] Verify no other import fixes needed in engine files (they don't cross-reference each other)

## CLI layer

- [x] Move collect.rs, compile.rs, list.rs into `src/cli/`
- [x] Fix cli/collect.rs imports: split across `crate::engine::` and `crate::domain::`
- [x] Fix cli/compile.rs imports: rewrite all `crate::agent`, `crate::config`, `crate::index`, `crate::leaf`, `crate::tree`, `crate::branch`, `crate::frontmatter`, `crate::slug`
- [x] Fix cli/list.rs imports: `crate::{frontmatter, index}` → `crate::domain::{frontmatter, index}`

## main.rs

- [x] Update `use bo::collect` → `use bo::cli::collect`
- [x] Update `use bo::config::{self, Config, ConfigError}` → `use bo::engine::config::{self, Config, ConfigError}`
- [x] Update `use bo::index` → `use bo::domain::index`
- [x] Update `use bo::list::{self, ListOptions}` → `use bo::cli::list::{self, ListOptions}`
- [x] Update `bo::config::TreeConfig` in cmd_seed → `bo::domain::tree::TreeConfig`
- [x] Update `bo::compile::cmd_compile` → `bo::cli::compile::cmd_compile`

## Test relocation

- [x] Move all tests + HTML constants from `tests/integration.rs` into `src/cli/collect.rs` `#[cfg(test)]`
- [x] Delete `tests/integration.rs`
- [x] Move 3 offline unit tests from `tests/integration_compile.rs` into `src/cli/compile.rs` `#[cfg(test)]`
- [x] Update remaining `tests/integration_compile.rs` paths: `bo::compile` → `bo::cli::compile`, `bo::config` → `bo::engine::config` / `bo::domain::tree`, `bo::index` → `bo::domain::index`, `bo::leaf` → `bo::domain::leaf`, `bo::frontmatter` → `bo::domain::frontmatter`
- [x] Update `tests/integration_cli.rs`: `bo::index::` → `bo::domain::index::`

## Verify

- [x] `cargo build` compiles without errors
- [x] `cargo test` — all tests pass
- [x] `cargo clippy` — no warnings
- [x] No `domain/` file imports from `cli/`, `engine/`, or `adapters/`
- [x] No `tests/*.rs` file calls library functions for single-module logic (those are now in-module)
