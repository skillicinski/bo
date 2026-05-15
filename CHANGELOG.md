# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Changed

- `bo raze` now preserves stored provider credentials by default; use `--include-auth` for a full credential wipe.

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
