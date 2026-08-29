# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Go workflows for seeding, snapping, reading state, and synthesizing a local workspace.
- Public workspace contracts for external Go backends.

### Changed

- **Breaking:** replaced the previous CLI with the Go CLI; install from source
  with `go install github.com/skillicinski/bo/cmd/bo@latest`.
- Local DeepSeek agent credentials now use `DEEPSEEK_API_KEY` and `DEEPSEEK_API_URL`.

The versioned entries below describe the pre-greenfield product and remain for
historical reference.

## [0.1.0] - 2026-07-19

### Changed

- **Breaking:** renamed the `bo compile` command to `bo synthesize`. The old command name is rejected; there is no alias.
- **Breaking:** renamed the `compile_model` config key and `--compile-model` flag to `synthesis_model` / `--synthesis-model`. `~/.bo/config.json` must be migrated by hand.
- **Breaking:** renamed serialized state field `last_compiled_at` to `last_synthesized_at` in `.bo/state.json`, status JSON, and status human output. Existing trees require manual migration.
- **Breaking:** renamed status leaf fields `uncompiled` / `uncompiled_slugs` to `unsynthesized` / `unsynthesized_slugs`, and `uncompiled` display wording to `unsynthesized`.
- **Breaking:** renamed transaction kind tag `Compile` to `Synthesize` in `.bo/pending.json` (the `kind_label`/recovery report now reads `synthesize`).
- **Breaking:** renamed the agent terminal tool `submit_compile` to `submit_synthesis`.
- **Breaking:** synthesis output status is now `synthesized` (was `compiled`); no-work reason is now `no new leaves since last synthesis`; validation error JSON phase is now `synthesis_validation`.
- **Breaking:** bumped the operation journal schema from 1 to 2 and renamed the journal op `compile` to `synthesize` in `.bo/journal.jsonl`. Old journals are not read across the schema boundary.
- **Breaking:** tree metadata and topology now live in `.bo/state.json` instead of `.bo/manifest.json`. Existing trees require manual migration before deploying v0.1.0; there is no automatic migration.
- **Breaking:** renamed machine-facing JSON fields and error codes to the new state vocabulary (e.g. `bo status` health `orphan_index_entries`/`missing_from_index` → `missing_leaf_files`/`untracked_leaf_files`; status state errors now report code `state_error` instead of `io_error`). The executable JSON envelope `schema_version` bumped from 1 to 2.
- **Breaking:** renamed JSON error code `pending_error` to `transaction_error` in collect responses. The `tree_busy` code is unchanged.

### Fixed

- Synthesis now resolves canonical `leaf/` paths and collision-disambiguated leaf slugs consistently when validating and recording branch membership, preventing false incremental drops and `leaf-`-prefixed stored slugs.

## [0.0.11] - 2026-07-18

### Removed

- `BO_RAZE_NON_INTERACTIVE` environment variable bypass.

### Changed

- `bo raze` now always requires an interactive terminal for confirmation and refuses to run non-interactively, including the credential-only `--include-auth` path when no tree is seeded.

### Fixed

- Two-stage `bo compile` stage one now sends the dedicated cluster system prompt it was budgeted against, instead of the generic compile prompt. Large-corpus clustering (full mode above 40 leaves, incremental above 15) now uses the intended prompt for cluster quality.

## [0.0.10] - 2026-07-13

### Added

- Two-stage compile for large corpora: above a leaf-count threshold (40 leaves in full mode, 15 new leaves in incremental mode) compile runs a cluster pass over titles and summaries — seeded by deterministic term-similarity candidate groupings — followed by one bounded synthesis call per cluster. Structural violations in the cluster response (duplicate assignments, unknown refs) are repaired in code with warnings instead of failing the run; a failed synthesis call retries once and is then dropped with a warning; all results commit as a single transaction. Below the threshold, single-pass compile is unchanged.
- `gpt-5.5` and `gpt-5.4` in the OpenAI model list (1,050,000-token context).
- Agent compile tools hardened: new `read_branch` tool (branch body + members), branch identifiers rendered distinctly as `branch/<slug>` and round-tripped on submission, teaching rejection messages ("X is a branch slug, not a leaf"), and the resource envelope stated in the system prompt.
- Budget-pressure signals in the agent loop: synthetic messages at 75% and 90% consumption telling the model to stop gathering and submit; `signals_sent` joins the diagnostics.
- Agent failure envelopes now carry full diagnostics: turns, tool calls, token usage, and the last tool/validation error.
- Journal records terminal compile errors (truncation, content filter, provider errors, context overflow) as `error: {code, message}` events, and two-stage runs record their stage counts.
- Agent engine: a provider-neutral, bounded multi-turn tool-calling loop in `engine::agent` with a typed tool contract, fixed resource envelope (turns, tool calls per response, total tool calls, per-call output tokens, context preflight, wall-clock), and explicit handling of truncation, context overflow, and the hard turn limit. Tool arguments are deserialized into typed structs at the tool boundary; `submit_compile` reuses the existing Full/Incremental validation gate and a valid submission terminates the loop. Provider quirks (tool_calls/tool_call_id preservation, `reasoning_content` replay after thinking-mode tool calls) are isolated in the OpenAI-compat adapter; the generic loop stays provider-neutral.
- `LlmProvider::complete_with_tools` extension with an explicit unsupported default; only the OpenAI-compat adapter (DeepSeek conformance target) implements it.
- `bo compile --dry-run`: a read-only validated preview that writes zero bytes — no pending recovery, no stale-branch repair, no commit. Captures the manifest hash at start and aborts if the tree changes before the preview is accepted.
- `bo compile --agent --dry-run`: the agent loop drives the compile preview through `list_leaves`, `list_branches`, `read_leaf`, `search_corpus`, and `submit_compile` tools, emitting the same preview contract with turn/tool-call/usage telemetry. `--agent` without `--dry-run` is rejected in this milestone.
- `--json` dry-run output includes provider/model, run mode, starting manifest hash, turns, tool-call count, optional provider token usage, and the full validated plan.
- Operation journal: `collect`, `compile`, `repair`, and `query` now append a best-effort event to `{tree}/.bo/journal.jsonl` (one JSON line per operation). `bo journal [--limit N]` reads the tail (newest last) in human or `--json` form. The journal is optional tier-3 state outside the manifest and write-lock transaction — a missing file is an empty journal and a journal failure never fails a command.

### Changed

- `CompileOptions` gained orthogonal `agent` and `dry_run` flags; `bo compile` without the new flags is unchanged.
- `bo compile --all` help and docs now state that a full recompile produces a new branch organization each run (a reroll, not a refresh).
- `docs/providers.md` gains compile-model guidance: cluster quality varies sharply by model; pin `--compile-model` to a validated model for corpora beyond the two-stage threshold.

## [0.0.9] - 2026-07-09

### Fixed

- The Google provider no longer rejects unknown model ids — any non-empty id is accepted (matching the custom provider), with the known-model table retained for context-window metadata and a 1M-token assumed window for unknown ids. Google retired the gemini-2.5 models on the Developer API with the hardcoded allowlist still pointing at them, leaving every google-provider configuration unusable. Known table now includes `gemini-3.5-flash`, `gemini-3.1-pro-preview`, and the floating `gemini-flash-latest` / `gemini-pro-latest` aliases.

## [0.0.8] - 2026-07-09

### Added

- Local markdown ingestion: `bo collect ./note.md` collects a local `.md` file as a note — frontmatter stripped (with a warning), title from a leading `# H1` when present, content-addressed source URL (`bo://note/<hash>`) so identical notes dedupe, no LLM summary call.

### Changed

- **breaking** tree layout: leaves now live in `leaf/` and branches in `branch/` (was root-level `.md` and `branches/`). Every domain entity gets its own singular directory. No migration — pre-user break.

### Fixed

- Leaf bodies no longer duplicate the title heading when the article's own H1 survives extraction (e.g. a page that leads with a byline before `# Title`). bo's prepended heading is skipped when the body already opens with a matching H1.
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

- npm distribution — install with `npm install -g @skillicinski/bo`. No compiler toolchain needed. Pre-built binaries for macOS (Intel + Apple Silicon) and Linux x86_64.
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
- Requires Go toolchain to install
