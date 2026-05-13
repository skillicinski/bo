# AGENTS.md

## What is bo?

Rust CLI tool. Collects web pages into a local markdown knowledge tree, compiles topic branches via LLM, and answers questions with citations over collected content.

## Architecture

Deterministic pipeline tool, not an autonomous agent. LLM commands (compile, query, summary) follow: code gathers context → one structured-output LLM call → code writes results. See `docs/adrs/001-deterministic-pipelines-over-internal-agent.md`.

## Project layout

```
src/
├── cli/          # CLI command implementations (collect, compile, config, list, search, show, query, raze, seed)
├── domain/       # Core types: leaf, branch, tree, index, slug, frontmatter
├── engine/       # Infrastructure: fetch, extract, config, auth, quality, summary, llm/
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
- `CHANGELOG.md` — user-facing changelog (Keep a Changelog format)
- `deny.toml` — cargo-deny config

## Conventions

- **Testing:** one test file per module in `src/tests/`. Run `cargo test`.
- **Linting:** `cargo clippy --all-targets --all-features -- -D warnings`
- **Formatting:** `cargo fmt`
- **No agent loops in bo itself** — LLM calls are single-shot structured output. Orchestration belongs to the calling agent, not bo.
- **`--json` flag** on all commands for machine consumption.
- **Config:** `~/.bo/config.json` — created by `bo seed` or `bo config set`.
- **Auth:** `~/.bo/auth.json` — created by `bo config auth`. Separate from config.

## Changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). When adding a release entry:

- Add a new `## [x.y.z] - YYYY-MM-DD` section at the top (below the header).
- Group changes under `### Added`, `### Changed`, `### Fixed`, `### Removed` as applicable.
- Write entries from the user's perspective, not implementation details.
- Keep entries concise — one line per change.

## Pull requests

PRs don't need "Verification", "Tests", or "How to test" sections. CI gates every merge and PRs go through human review. A brief summary of what changed and why is sufficient.

## Releasing

1. Update `CHANGELOG.md` with the new version section.
2. Bump `version` in `Cargo.toml`.
3. Commit, merge to main.
4. `git tag v<version> && git push --tags`

The `release.yml` workflow runs CI and creates a GitHub Release with notes extracted from CHANGELOG.md.

## Current state (v0.0.1)

Commands shipping: `seed`, `collect`, `list`, `search`, `show`, `query`, `compile`, `config`, `raze`.

## LLM provider

OpenAI-compatible only (for now). Auth resolved via: `OPENAI_API_KEY` env var → `~/.bo/auth.json` → error. Single `model` config field (default `gpt-4o`), configurable via `bo config set model`.
