# AGENTS.md

## What is bo?

Rust CLI tool. Collects web pages into a local markdown knowledge tree, compiles topic branches via LLMs, and answers questions with citations over collected content.

## Project layout

Before performing broad `read` and `bash` tool calls, use the below information to get a sense of where to find what you are looking for.

- `src/cli/` — CLI command implementations (collect, compile, config, list, show, query, raze, seed)
- `src/domain/` — Core types: leaf, branch, tree, manifest, slug, frontmatter
- `src/engine/` — Infrastructure: fetch, extract, config, auth, quality, summary, llm/
- `src/adapters/` — Source-specific adapters (youtube/)
- `src/tests/` — Unit tests (one file per module)
- `tests/` — Integration tests (architecture, CLI end-to-end, cross-module scenarios)
- `src/lib.rs` — Library root (re-exports)
- `src/main.rs` — CLI entry point, argument parsing, output formatting
- `npm/` — npm wrapper package (install.js, run.js, package.json)
- `.github/workflows/release.yml` — CI pipeline: gate → build matrix → GitHub Release → npm stage publish
- `CHANGELOG.md` — User-facing changelog (Keep a Changelog format)
- `docs/usage.md` — Usage guide with command walkthrough and examples

Project planning, ADRs, specs, and session notes live in `internal/` (gitignored).

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

This project follows [Semantic Versioning](https://semver.org/).

1. Update `CHANGELOG.md` with the new version section.
2. Bump `version` in `Cargo.toml`.
3. Bump `version` in `npm/package.json` to match.
4. Commit, merge to main.
5. `git tag v<version> && git push --tags`

The `release.yml` workflow runs CI (format, clippy, deny, test), builds platform binaries for macOS Intel/Apple Silicon and Linux x86_64, creates a GitHub Release with the tarballs attached, and publishes `@skillicinski/bo` to npm.

## LLM providers

See [docs/providers.md](docs/providers.md) for per-provider model tables and API-specific behaviour.
