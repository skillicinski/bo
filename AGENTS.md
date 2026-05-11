# AGENTS.md

## What is bo?

Rust CLI tool. Collects web pages into a local markdown knowledge tree, compiles topic branches via LLM, and answers questions with citations over collected content.

## Architecture

Deterministic pipeline tool, not an autonomous agent. LLM commands (compile, query, summary) follow: code gathers context → one structured-output LLM call → code writes results. See `docs/adrs/001-deterministic-pipelines-over-internal-agent.md`.

## Project layout

```
src/
├── cli/          # CLI command implementations (collect, compile, list, search, show, query, raze, seed)
├── domain/       # Core types: leaf, branch, tree, index, slug, frontmatter
├── engine/       # Infrastructure: fetch, extract, config, quality, summary, llm/
├── adapters/     # Source-specific adapters (youtube/)
├── tests/        # Unit/integration tests (one file per module)
├── lib.rs        # Library root (re-exports)
└── main.rs       # CLI entry point, argument parsing, output formatting
```

## Key paths

- `docs/adrs/` — architectural decision records (tracked)
- `docs/milestones/` — release roadmap and backlog (tracked)
- `docs/scratchpad/` — session notes, idea capture (gitignored)
- `docs/specs/` — feature implementation specs (gitignored)
- `deny.toml` — cargo-deny config

## Conventions

- **Testing:** one test file per module in `src/tests/`. Run `cargo test`.
- **Linting:** `cargo clippy --all-targets --all-features -- -D warnings`
- **Formatting:** `cargo fmt`
- **No agent loops in bo itself** — LLM calls are single-shot structured output. Orchestration belongs to the calling agent, not bo.
- **`--json` flag** on all commands for machine consumption.
- **Config:** `~/.bo/config.json` — created by `bo seed`.

## Current state (v0.0.1 in progress)

Commands shipping: `seed`, `collect`, `list`, `search`, `show`, `query`, `compile`, `raze`.

Release backlog: `docs/milestones/oss-release-backlog.md`

## LLM provider

OpenAI-compatible only (for now). Requires `OPENAI_API_KEY` env var or `.env` file. Model configurable via config JSON (`compile_model`, `query_model`).
