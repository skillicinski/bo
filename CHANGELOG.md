# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `google` provider — Gemini API support via `bo config --provider google --model gemini-2.5-flash`. Includes `gemini-2.5-flash-lite`, `gemini-2.5-flash`, and `gemini-2.5-pro` models with 1M token context windows. Auth via `GEMINI_API_KEY` env var or `google_api_key` in `~/.bo/auth.json`.

## [0.0.4] - 2026-05-26

### Fixed

- `bo collect` — LLM summary generation no longer falls back to the deterministic summary on OpenAI providers. The structured-output schema was being rejected by OpenAI's strict-mode validator because `summary` was missing from the schema's `required` list. DeepSeek users were unaffected.

## [0.0.3] - 2026-05-26

### Fixed

- npm install — `bo` no longer fails with `use strict: command not found` / `syntax error near unexpected token '('` after `npm install -g @skillicinski/bo`. The wrapper script was missing its `#!/usr/bin/env node` shebang, causing the OS to execute it as a shell script.

## [0.0.2] - 2026-05-25

### Added

- npm distribution — install with `npm install -g @skillicinski/bo`. No Rust toolchain needed. Pre-built binaries for macOS (Intel + Apple Silicon) and Linux x86_64.
- DeepSeek provider support (`deepseek-v4-flash`, `deepseek-v4-pro`). Select with `bo config --provider deepseek`.
- `bo status` command — shows tree health and compile readiness.
- `bo compile --all` flag — recompile the full corpus and allow complete branch graph rewrite.
- `bo collect` accepts multiple URLs in one invocation and `.txt` files containing one URL per line.
- `compile_model` config key — pin a heavier model just for `bo compile`. Set with `bo config --compile-model <id>`.
- Compile validation gate — `bo compile` rejects responses that would result in no file changes and surfaces an actionable error.
- Interactive confirmation gate on `bo raze` — operators must type `yes` to proceed. No `--force` / `--yes` override (by design, so agents cannot bypass it).
- `manifest.json` is now the canonical record of tree topology and metadata. Lives at `{tree}/.bo/manifest.json`. Written atomically (tmp + rename) on every mutation.

### Changed

- `bo compile` is incremental by default — only processes leaves collected since the last compile. Use `--all` for full recompile.
- `bo config` is now flag-driven: `bo config --provider <name> --model <id> --compile-model <id>`. Replaces the previous `bo config auth` / `bo config get` / `bo config set` subcommands.
- Default model changed from `gpt-4o` to `gpt-4.1-mini`.
- `bo show` now defaults to a frontmatter card view; use `--full` for the complete body.
- `bo list` reworked into a branch-centric tree view by default. New flags: `--branch <name>`, `--recent`, `--limit <n>`.
- All read commands (`bo status`, `bo list`, `bo show`, `bo query`, plus `bo collect` duplicate detection) consult the manifest. They no longer read `index.jsonl`, `state.json`, or scan the `branches/` directory for metadata.
- `bo collect` and `bo compile` write the manifest as the primary commit, then mirror to `index.jsonl` / `state.json` / branch frontmatter as a transient safety net (removed in v0.0.2 stage 3b).
- Branch `.md` frontmatter renamed `compiled_at` → `created_at` for clarity. `updated_at` unchanged. The manifest's `BranchRecord` mirrors these names.
- `bo raze` now preserves stored provider credentials by default; use `--include-auth` for a full credential wipe.
- `bo status` no longer counts leaves with unparseable frontmatter as "uncompiled". Such leaves were always also flagged as `skipped` by health output; the duplicate signal is gone, the explicit `skipped` signal stays.
- Compile context-overflow and validation errors now reference the new flag-based config (`bo config --compile-model <id>` / `bo config --model <id>`).

### Removed

- `bo search` command. Use `bo list --terms <terms>` for title/slug filtering.
- `bo config auth` subcommand. Save API keys via the `OPENAI_API_KEY` / `DEEPSEEK_API_KEY` env vars or by hand-editing `~/.bo/auth.json` with flat keys (`openai_api_key`, `deepseek_api_key`).
- `bo config get` and `bo config set` subcommands. Replaced by the flag-driven `bo config`.

### Recovery

- If `manifest.json` is missing on read, bo reconstructs it from the secondary store (`index.jsonl` + branch frontmatter) on the fly, persists the result, and prints `manifest missing; reconstructed from secondary store` to stderr. This affordance is removed in v0.0.2 stage 3b along with the secondary store itself.

## [0.0.1] - 2026-05-13

First experimental release.

### Added

- `bo seed`, `bo collect`, `bo compile`, `bo query`, `bo list`, `bo search`, `bo show`, `bo raze`
- `bo config auth --provider openai` — store API key locally
- `bo config set model` / `bo config get model`
- `--json` flag on all commands
- YouTube transcript collection
- Zero-citation detection (refuses hallucinated answers)
- Install smoke test in CI

### Notes

- OpenAI-compatible providers only
- Lexical retrieval (no embeddings)
- Requires Rust toolchain to install
