# Plan: Organise src/ by architectural layers

## Architecture decisions

1. **Four top-level modules**: `domain/`, `cli/`, `engine/`, `adapters/` (already exists).
2. **TreeConfig moves to domain/tree.rs** — it's a pure data struct consumed by `Tree::from_config`. This resolves a dependency-direction violation (`domain` must not import from `engine`). `engine/config.rs` will import `TreeConfig` from domain instead.
3. **No re-exports at crate root** — `lib.rs` declares four `pub mod` entries. Consumers use fully-qualified paths.
4. **adapters/ stays at crate root** — it's a peer to the other layers, consumed by engine, not owned by it.

## File moves

| Source | Destination | Layer |
|--------|-------------|-------|
| `src/tree.rs` | `src/domain/tree.rs` | domain |
| `src/branch.rs` | `src/domain/branch.rs` | domain |
| `src/leaf.rs` | `src/domain/leaf.rs` | domain |
| `src/frontmatter.rs` | `src/domain/frontmatter.rs` | domain |
| `src/slug.rs` | `src/domain/slug.rs` | domain |
| `src/index.rs` | `src/domain/index.rs` | domain |
| `src/collect.rs` | `src/cli/collect.rs` | cli |
| `src/compile.rs` | `src/cli/compile.rs` | cli |
| `src/list.rs` | `src/cli/list.rs` | cli |
| `src/fetch.rs` | `src/engine/fetch.rs` | engine |
| `src/extract.rs` | `src/engine/extract.rs` | engine |
| `src/quality.rs` | `src/engine/quality.rs` | engine |
| `src/agent.rs` | `src/engine/agent.rs` | engine |
| `src/config.rs` | `src/engine/config.rs` | engine |
| `src/adapters/` | `src/adapters/` (unchanged) | adapters |

## New files to create

- `src/domain/mod.rs` — declares: tree, branch, leaf, frontmatter, slug, index
- `src/cli/mod.rs` — declares: collect, compile, list
- `src/engine/mod.rs` — declares: fetch, extract, quality, agent, config

## Import rewrites

### domain/branch.rs
- `use crate::frontmatter` → `use crate::domain::frontmatter`

### domain/leaf.rs
- `use crate::frontmatter::{self, FrontmatterError}` → `use crate::domain::frontmatter::{self, FrontmatterError}`

### domain/tree.rs
- Remove `use crate::config::TreeConfig` (TreeConfig moves here)

### engine/config.rs
- Add `use crate::domain::tree::TreeConfig`
- Remove `TreeConfig` struct definition (moved to domain/tree.rs)

### cli/collect.rs
- `use crate::{extract, fetch, index, leaf, quality, slug, RejectReason}` → split across layers:
  - `use crate::engine::{extract, fetch, quality}`
  - `use crate::engine::quality::RejectReason`
  - `use crate::domain::{index, leaf, slug}`
- `use crate::adapters::youtube::{self, YoutubeError, YoutubeUrlMatch}` → unchanged

### cli/compile.rs
- `use crate::{branch, frontmatter, slug}` → `use crate::domain::{branch, frontmatter, slug}`
- `use crate::agent::{AgentConfig, OpenAiProvider}` → `use crate::engine::agent::{AgentConfig, OpenAiProvider}`
- `use crate::agent::{AgentError, Tool}` → `use crate::engine::agent::{AgentError, Tool}`
- `use crate::config::Config` → `use crate::engine::config::Config`
- `use crate::index` → `use crate::domain::index`
- `use crate::index::IndexEntry` → `use crate::domain::index::IndexEntry`
- `use crate::leaf` → `use crate::domain::leaf`
- `use crate::tree::Tree` → `use crate::domain::tree::Tree`

### cli/list.rs
- `use crate::{frontmatter, index}` → `use crate::domain::{frontmatter, index}`

### main.rs
- `use bo::collect` → `use bo::cli::collect`
- `use bo::config::{self, Config, ConfigError}` → `use bo::engine::config::{self, Config, ConfigError}`
- `use bo::index` → `use bo::domain::index`
- `use bo::list::{self, ListOptions}` → `use bo::cli::list::{self, ListOptions}`
- `bo::config::TreeConfig` (in cmd_seed body) → `bo::domain::tree::TreeConfig`
- `bo::compile::cmd_compile` → `bo::cli::compile::cmd_compile`

### lib.rs (full rewrite)
```rust
pub mod adapters;
pub mod cli;
pub mod domain;
pub mod engine;
```

## Implementation order

1. Create directories and `mod.rs` files
2. Move `TreeConfig` from config.rs → tree.rs
3. Move all files to new locations
4. Rewrite `lib.rs`
5. Fix all `use` paths
6. `cargo build` — iterate on any missed paths
7. `cargo test` — confirm green
8. `cargo clippy` — confirm no warnings

## Risks

- **Missed import path**: mechanical but numerous. Build will catch them immediately.
- **Visibility**: modules currently `pub` from crate root need to remain accessible via the new paths. Using `pub mod` in each layer's `mod.rs` preserves this.
- **External consumers**: none exist (binary-only crate), so API path changes have zero blast radius.

## Test relocation

### tests/integration.rs → src/cli/collect.rs #[cfg(test)]

All tests + HTML fixture constants move into `cli/collect.rs`. They test
`collect_html` directly (a function defined in that module) with injected HTML.
No path rewrites needed — they become `super::*` imports.

### tests/integration_compile.rs — split

**Move to src/cli/compile.rs #[cfg(test)]:**
- `compile_exits_cleanly_on_empty_collection`
- `compile_exits_cleanly_on_single_leaf`
- `compile_errors_without_api_key`

These are offline unit tests for guard-clause logic.

**Keep in tests/integration_compile.rs (update paths):**
- `compile_creates_branches_directory`
- `compile_produces_at_least_one_branch_file`
- `compile_gives_every_leaf_a_branches_field`
- `compile_does_not_modify_index_jsonl`
- `compile_rerun_preserves_compiled_at`
- `setup_fixture_collection` helper + `make_config` helper

Path rewrites:
- `bo::compile` → `bo::cli::compile`
- `bo::config::Config` → `bo::engine::config::Config`
- `bo::config::TreeConfig` → `bo::domain::tree::TreeConfig`
- `bo::index` → `bo::domain::index`
- `bo::leaf::write` → `bo::domain::leaf::write`
- `bo::frontmatter::parse` → `bo::domain::frontmatter::parse`

### tests/integration_cli.rs — update fixture paths

- `bo::index::append_entry` → `bo::domain::index::append_entry`
- `bo::index::IndexEntry` → `bo::domain::index::IndexEntry`

### tests/integration_network.rs — no changes

Zero `bo::` imports. Already invokes the binary via `Command`.
