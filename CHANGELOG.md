# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- Code blocks in collected articles are now preserved as fenced markdown instead of being flattened into single-line inline code spans. Multi-line `<pre>`/`<code>` blocks keep their line breaks.

## [0.0.7] - 2026-07-06

### Added

- `custom` provider — point bo at any OpenAI-compatible endpoint via `bo config --provider custom --base-url <url> --model <model>`. Any non-empty model id is accepted. Auth via `CUSTOM_API_KEY` env var or `custom_api_key` in `~/.bo/auth.json`.

### Changed

- Compile and collect diagnostics (title-collision warnings, branch-repair notices, pending-recovery notices, per-branch write progress) now print to stderr after the run completes instead of interleaved mid-run. Content and destination unchanged — only the timing shifts.

### Fixed

- Tracing/diagnostic output now routes to stderr, keeping stdout clean for command output and `--json` parsing.

### Architecture (non-user-facing)

- The engine is now the public library surface — a vocabulary of capabilities named for what they do, never for the command that invokes them. Query's retrieval, NLP, relevance, context-assembly, and citation-validation layers moved from `cli` to `engine::retrieval` with a semantic `RetrievalError` mapped at the CLI boundary.
- Presentation purity across all workflows: nothing below a command's entry point touches stdout/stderr; workflows return typed results and the CLI renders them. A TUI/REPL/web front-end can now compose the same primitives.
- Compile pipeline stages completed: validation extracted into a dedicated I/O-free trust-boundary module owning `CompilePlan`, and `repair_stale_branches` decomposed into a classify → prune → repair → assemble pipeline.

## [0.0.6] - 2026-07-05

### Added

- `zai` provider — Z.ai (GLM) API support via the GLM Coding Plan endpoint (`https://api.z.ai/api/coding/paas/v4`). Includes `glm-4.7` (default), `glm-4.5-air`, `glm-5.1`, `glm-5-turbo`, and `glm-5.2` models. Auth via `ZAI_API_KEY` env var or `zai_api_key` in `~/.bo/auth.json`.

### Changed

- Untitled leaves now report `title: null` instead of an empty string in JSON output (`bo show`, `bo status`, `bo query --json`) and in the manifest. Existing manifests load unchanged.

### Architecture (non-user-facing)

- Tree/branch/leaf metaphor committed to types — `Leaf`/`Branch` domain entities (was `LeafRecord`/`BranchRecord`), validated `Title`/`Url` newtypes replace bare `String` aliases.
- Frontmatter serialization unified on typed structs (`LeafFrontmatter`/`BranchFrontmatter`) with golden-tested byte-stable `.md` file output.
- On-disk formats (`manifest.json`, `config.json`, `.md` frontmatter) unchanged — this is an internal type-strengthening, not a format migration.

## [0.0.5] - 2026-06-30

### Added

- `google` provider — Gemini API support via `bo config --provider google --model gemini-2.5-flash`. Includes `gemini-2.5-flash-lite`, `gemini-2.5-flash`, and `gemini-2.5-pro` models with 1M token context windows. Auth via `GEMINI_API_KEY` env var or `google_api_key` in `~/.bo/auth.json`.
- `bo query` now retrieves compiled branches in addition to raw leaves — the LLM sees synthesized concept pages alongside source documents and may cite either kind.
- Compile degenerate-result warning: when a Full compile on a large corpus produces a suspiciously thin branch set, bo warns the user without blocking the output. Retry with `--all` or switch models if the warning appears consistently.
- Architecture documentation: `docs/architecture.md` (design decisions) and `src/AGENTS.md` (code rules for contributors / coding agents).
- `deepseek-v4-pro` model entry (1M context, stronger than v4-flash).

### Changed

- Fresh trees auto-run Full compile mode. Incremental mode now only activates when branches already exist, fixing a class of validation errors where the LLM was asked to update non-existent branches.
- `bo seed` no longer silently defaults to gpt-4.1-mini when no model is given. The model field is required — either via the `--model` flag or the interactive prompt. `bo config` without a model now fails with a clear error instead of silently falling back.
- Compile validation hardened: ambiguous leaf title references (two leaves sharing a title) are now rejected rather than silently resolving to the wrong leaf. Leaf slugs are included in the full-compile prompt so models see the canonical identifier.
- Schema generation uses `schemars`-derived inline schemas (no `$ref`/`definitions`/$schema`) — fixes a Google provider rejection and keeps schemas portable across all three providers.
- Type-level schema enforcement: the `LlmProvider` contract now requires normalized schemas at compile time, not runtime.
- Compile output (`--json`) no longer reports a redundant `context_mode` field (it was always 1:1 with `mode`).
- Three different LLM runtime patterns unified behind a shared `OnceLock`-backed runtime. All CLI modules share one long-lived async executor rather than building fresh ones per call.

### Fixed

- Branch `.md` frontmatter now correctly drops deleted leaf filenames from the `leaves:` list when a leaf is removed. Previously the manifest was updated but the branch file on disk was stale.
- Google provider compile returning HTTP 400 on schema fields injected by `schemars` — fixed by inlining subschemas.
- Duplicate-URL detection on single-URL `bo collect` — a regression from an internal refactor that dropped the pre-fetch duplicate check.
- Five distinct compile failure modes are now caught by the validation gate and refuse to write, rather than silently producing a corrupted or degraded tree.

### Architecture (non-user-facing)

- Compile pipeline refactored: mode-delta logic split into focused functions, duplicated preflight collapsed, `CompileContextMode` redundant type removed. God-function eliminated; pipeline reads as a traceable progression.
- Retrieval scoring deduplicated: leaf and branch scoring share a single parameterized function.
- Atomic manifest-commit sequence extracted from duplicate collect/compile sites into a shared `engine::pending` helper.
- `pub(super)` visibility discipline enforced throughout compile and query modules — no internal plumbing leaks across commands.

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
