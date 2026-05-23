# AGENTS.md

## What is bo?

Rust CLI tool. Collects web pages into a local markdown knowledge tree, compiles topic branches via LLM, and answers questions with citations over collected content.

## Architecture

Deterministic pipeline tool, not an autonomous agent. LLM commands (compile, query, summary) follow: code gathers context → one structured-output LLM call → code writes results. See `docs/adrs/001.md`.

Architectural decisions are recorded in `docs/adrs/`. Consult them when making implementation decisions — especially ADR-001 (pipeline boundaries), ADR-002 (structured output schemas), ADR-004 (manifest design), and ADR-005 (deterministic processing at LLM boundaries).

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
- **Config:** `~/.bo/config.json` — created by `bo seed` or `bo config --provider/--model/--compile-model`.
- **Auth:** `~/.bo/auth.json` — flat keys `openai_api_key` / `deepseek_api_key`. Hand-edited or set via env var. Separate from config.

## Changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). When adding a release entry:

- Add a new `## [x.y.z] - YYYY-MM-DD` section at the top (below the header).
- Group changes under `### Added`, `### Changed`, `### Fixed`, `### Removed` as applicable.
- Write entries from the user's perspective, not implementation details.
- Keep entries concise — one line per change.

## Pull requests

PRs don't need "Verification", "Tests", or "How to test" sections. CI gates every merge and PRs go through human review. A brief summary of what changed and why is sufficient.

When creating new PRs, the body should begin with a itemised list that describe the core functional changes to the project. Each item should begin with a null-subject verb, eg. "added", "removed", "updated", "refactored", "bumped" to describe the type of operation on the code, followed by a one-line summary of the change itself.

Link related issues and stacked PRs at the bottom of the PR body, separated from the summary by `---` on its own line:
- `Closes #<number>` for issues this PR resolves
- `Stacked on #<number>` for an open PR this builds on top of

## Releasing

1. Update `CHANGELOG.md` with the new version section.
2. Bump `version` in `Cargo.toml`.
3. Commit, merge to main.
4. `git tag v<version> && git push --tags`

The `release.yml` workflow runs CI and creates a GitHub Release with notes extracted from CHANGELOG.md.

## Current state (v0.0.1)

Commands shipping: `seed`, `collect`, `list`, `show`, `query`, `compile`, `config`, `status`, `raze`.

## LLM provider

OpenAI-compatible only (for now). Two providers: `openai` (default) and `deepseek`. Auth resolved via provider-specific env var (`OPENAI_API_KEY` / `DEEPSEEK_API_KEY`) → `~/.bo/auth.json` → error. Models configured via `bo config --model <id>` (and optional `--compile-model <id>`). Default model: `gpt-4.1-mini`.
