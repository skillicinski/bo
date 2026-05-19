# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- `manifest.json` is now the canonical record of tree topology and metadata. Lives at `{tree}/.bo/manifest.json`. Written atomically (tmp + rename) on every mutation.

### Changed

- All read commands (`bo status`, `bo list`, `bo show`, `bo query`, `bo search`, plus `bo collect` duplicate detection) consult the manifest. They no longer read `index.jsonl`, `state.json`, or scan the `branches/` directory for metadata.
- `bo collect` and `bo compile` write the manifest as the primary commit, then mirror to `index.jsonl` / `state.json` / branch frontmatter as a transient safety net (removed in v0.0.2 stage 3b).
- Branch `.md` frontmatter renamed `compiled_at` → `created_at` for clarity. `updated_at` unchanged. The manifest's `BranchRecord` mirrors these names.
- `bo raze` now preserves stored provider credentials by default; use `--include-auth` for a full credential wipe.
- `bo status` no longer counts leaves with unparseable frontmatter as "uncompiled". Such leaves were always also flagged as `skipped` by health output; the duplicate signal is gone, the explicit `skipped` signal stays.

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
