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

## Development

- Always add a `--json` flag to new user-facing commands.
- No agent loops in bo — LLM calls are single-shot structured output. Orchestration belongs to the calling agent.
- One test file per module in `src/tests/`; integration tests in `tests/`.

## Changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). When adding a release entry:

- Add a new `## [x.y.z] - YYYY-MM-DD` section at the top (below the header).
- Group changes under `### Added`, `### Changed`, `### Fixed`, `### Removed` as applicable.
- Write entries from the user's perspective, not implementation details.
- Keep entries concise — one line per change.

## Pull requests

PRs don't need "Verification", "Tests", or "How to test" sections. CI gates every merge and PRs go through human review. A brief summary of what changed and why is sufficient.

When creating new PRs, the body should begin with a itemised list that describes the core functional changes to the project. Items are formatted use the unordered list markdown syntax. Each item should begin with a null-subject verb, eg. "added", "removed", "updated", "refactored", "bumped" to describe the type of operation on the code, followed by a one-line summary of the change itself.

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
6. **Approve the `npm-publish` GitHub Environment** — Actions tab → tag run → click *Review deployments* → approve. The job will not run until you do.
7. **Approve the staged tarball on npmjs.com** — `npm stage publish` uploads to a staging queue, not to public. Open the package page on npmjs.com from a 2FA-trusted device and approve the pending stage.

The `release.yml` workflow runs CI (format, clippy, deny, test), builds platform binaries for macOS Intel/Apple Silicon and Linux x86_64, creates a GitHub Release with the tarballs attached, and stages `@skillicinski/bo` to npm via OIDC trusted publishing.

### Why two human gates?

The environment approval (step 6) blocks token issuance: the OIDC token GitHub mints carries the `environment: npm-publish` claim, which npm's trusted-publisher config requires. The staged-publish approval (step 7) blocks public availability: the tarball is uploaded but invisible until a maintainer signs off from a trusted device. Either gate alone would not stop a compromised CI from publishing.

### Recovering from a failed release

- If `npm-publish` failed (e.g. before this hardening), delete the partial release and re-tag at a fixed commit:
  ```bash
  gh release delete v<version> --yes --cleanup-tag
  git tag -d v<version>
  git tag v<version> <fixed-commit>
  git push origin v<version>
  ```
- Versions that successfully publish to npm are **permanently burned** even after `npm unpublish`. Bump to the next patch instead of reusing.
- Re-running a tag-triggered workflow replays the workflow YAML from the tag's commit, not from `main`. If the fix is on `main`, you must re-tag, not just re-run.

## LLM providers

See [docs/providers.md](docs/providers.md) for per-provider model tables and API-specific behaviour.
