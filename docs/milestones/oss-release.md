# OSS Release Milestone

North-star vision for the path from dogfood alpha to public open-source release. This document captures strategic framing, readiness gates, and sequencing constraints. It is not a living backlog — see `scratchpad/features.md` for right-sized implementation candidates.

## Target loop

```text
collect → compile → inspect → query → refine → repeat
```

Each stage is itself a loop:

- **collect** — gather URLs over time; each `bo collect` adds a leaf. The tree grows incrementally.
- **compile** — run after any batch of new leaves to rebuild branches and cross-references. Repeatable and idempotent.
- **inspect** — `list`, `show`, `search` to see what's in the tree, check branches, spot gaps. No network, no keys.
- **query** — ask questions, get cited answers from your tree. Wire into a coding agent via CLI/JSON for local RAG.
- **refine** — prune bad leaves, collect more, recompile, query again. The tree improves through use.

## Storage philosophy

Markdown tree is user-owned and inspectable. Any DB, index, or snapshot layer is acceleration and safety — never the source of truth. The tree must be rebuildable from its own content.

## Architectural stance

Bo is a deterministic pipeline tool, not an autonomous agent. LLM-powered commands (compile, query, lint) follow a fixed pattern: code gathers context → one structured-output LLM call → code writes results. Bo does not contain orchestration or agent-loop logic.

Bo's CLI commands are reliable primitives. External agents (Claude Code, Cursor, custom setups) compose bo commands to automate knowledge base workflows. An MCP server is a natural extension of the CLI for agent consumption. See `adrs/001-deterministic-pipelines-over-internal-agent.md`.

## v0.0.1 — Experimental OSS release

Committed scope. Ship this, get feedback, iterate.

### 1. Housekeeping (done)
- [x] LICENSE file in repo root (MIT, matching Cargo.toml)
- [x] Cargo.toml metadata: description, repository, homepage, keywords
- [x] CI workflow: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`

### 2. Zero-citation = not-answered (done)
- [x] If synthesis produces zero valid citations, treat result as "not answered from collected sources"
- [x] Surface clear human (`no answer from collected sources`) and JSON (`insufficient_sources`) behavior
- [x] Prevents hallucination when irrelevant OR-matched leaves reach synthesis

### 3. `bo config set` MVP (done)
- [x] `bo config set model <value>` — set the global LLM model
- [x] `bo config get model` — show current effective model
- [x] Removes requirement to hand-edit `~/.bo/config.json` for model selection

### 4. Clean installation happy path (release gate, separate branch)
- [ ] New user can install bo from GitHub without cloning the repo for contribution work
- [ ] Document the release install command: `cargo install --git https://github.com/jellysnake/bo --tag v0.0.1`
- [ ] Installed `bo` works from any local shell / arbitrary working directory, not just inside the repository
- [ ] Validate at minimum: `bo --help`, `bo config get model`, and `bo seed <tree-dir>` from a temp `HOME`
- [ ] Keep crates.io/Homebrew/binary artifacts out of v0.0.1 unless they become trivial; those remain post-release packaging

### 5. Auth configuration happy path (release gate, separate branch)
- [ ] `bo config auth --provider openai` prompts for an OpenAI API key without echoing it
- [ ] Auth command writes/updates `~/.bo/auth.json` and creates `~/.bo` if needed
- [ ] Stored auth file uses restrictive local permissions where supported (`0600` on Unix/macOS)
- [ ] Re-running the auth command overwrites the existing OpenAI key without printing it
- [ ] LLM-backed commands resolve API keys in order: `OPENAI_API_KEY` env var, then `~/.bo/auth.json`, then clear setup error
- [ ] Error messages point users to `bo config auth --provider openai` instead of requiring `.env`/`source .env`
- [ ] JSON mode works for success/errors and never includes the secret value

### 6. README rewrite + tag (capstone session)
- [ ] What bo is / is not
- [ ] Install (`cargo install --git https://github.com/jellysnake/bo --tag v0.0.1`; future crates.io once published)
- [ ] Quickstart walkthrough (install → auth → seed → collect → list → compile → query)
- [ ] Command reference
- [ ] BYOK / provider setup (`bo config auth --provider openai`, `OPENAI_API_KEY` override, `bo config set model`)
- [ ] Storage format overview
- [ ] Limitations + experimental caveat
- [ ] Tag v0.0.1, push

### Already done (for reference)
- [x] Inspect commands: `list`, `show`, `search`
- [x] `bo query` V1 with citations
- [x] `bo compile` structured-output pipeline
- [x] YouTube adapter
- [x] Low-value rejection
- [x] `--json` on all commands
- [x] Leaf summaries

---

## Post-v0.0.1 — next increments

Prioritize based on user feedback. Candidates (not committed):

- Tree health scan/survey (`bo scan`)
- Index rebuild (`bo rebuild-index`)
- Compile dry-run / snapshot
- Local/OpenAI-compatible endpoint support
- Query no-answer hardening beyond zero-citation (retrieval relevance floor)
- PDF adapter
- RSS feed collection
- Source-adapter hardening (Medium, xcancel)
- Dogfood regression expectations
- Release packaging (crates.io publish, binaries)

## Sequencing principles

1. Deterministic inspection before LLM-dependent features — users and agents must be able to see and search tree state without network or provider keys.
2. Read-only retrieval before mutation — query should not modify the tree; compile safety should gate writes.
3. Lexical/deterministic search before vector/embedding search — avoid DB infrastructure until the simple path is proven insufficient.
4. One command per session — promote and implement one right-sized feature at a time from `scratchpad/features.md`.
5. Adapter breadth is lower priority than core loop completeness — don't add new source adapters until inspect/query/compile safety exist.
6. Bo is a tool, not an agent — LLM calls are single-shot structured output within deterministic pipelines; orchestration belongs to the user or their agent.

## Implementation candidates

See `scratchpad/features.md` for the authoritative backlog of right-sized feature candidates.
