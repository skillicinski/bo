# v0.0.1 Release Backlog

Ordered implementation work for the Experimental OSS release. Each section is one session of work. Items map directly to gates in `milestones/oss-release.md`.

---

## 1. Housekeeping

**Goal:** Mechanical release infrastructure. No design decisions.

- [ ] Add `LICENSE` file (MIT, full text) to repo root
- [ ] Update `Cargo.toml` metadata:
  - `description` — one-line (e.g. "Collect web pages into a local markdown knowledge tree")
  - `repository` — GitHub URL
  - `homepage` — GitHub URL (or separate if exists)
  - `keywords` — e.g. `["cli", "knowledge-base", "markdown", "rag", "web"]`
  - `categories` — e.g. `["command-line-utilities"]`
- [ ] Add GitHub Actions CI workflow (`.github/workflows/ci.yml`):
  - `cargo fmt --check`
  - `cargo clippy --all-targets --all-features -- -D warnings`
  - `cargo test`
  - Trigger on push to `main` and PRs
  - Rust stable toolchain

**Done when:** CI is green on main, LICENSE renders on GitHub, `cargo package --list` includes LICENSE.

---

## 2. Zero-citation = not-answered

**Goal:** If query synthesis produces zero valid citations, treat the result as unanswered rather than emitting a hallucinated response.

**Context (from dogfood):** V1's lexical OR retrieval can pull generic-term matches for unrelated questions. The model usually refuses, but can answer from parametric knowledge with zero citations (observed: PMNS matrix query). The zero-citation patch is an answerability detection fix, not a retrieval quality fix.

**Implementation:**

- [ ] After synthesis, check `cited_slugs` (the validated citation list)
- [ ] If `cited_slugs` is empty:
  - Human output: print "no answer from collected sources" (or similar), exit 1
  - JSON output: `{ "status": "error", "error": { "code": "insufficient_sources", "message": "..." } }`
- [ ] Add unit tests:
  - Mock synthesis that returns prose with zero valid wikilinks → verify not-answered behavior
  - Mock synthesis that returns prose with ≥1 valid wikilink → verify normal answer output
- [ ] Add integration test: query against a tree with irrelevant content for a known-unrelated question → verify exit 1

**Done when:** `bo query "What is the PMNS matrix?"` against the default corpus exits 1 with clear messaging instead of emitting a hallucinated answer.

---

## 3. `bo config set` MVP

**Goal:** Users can change `compile_model` and `query_model` without hand-editing JSON.

**Implementation:**

- [ ] Add `Config` subcommand to CLI:
  ```
  bo config set <key> <value>
  bo config get <key>
  ```
- [ ] Supported keys (v0.0.1): `compile_model`, `query_model`
- [ ] `set` reads existing config, updates the field, writes back
- [ ] `get` reads config and prints the current value (or default if unset)
- [ ] Unknown key → exit 2 with error listing valid keys
- [ ] `--json` support on both subcommands
- [ ] Unit tests:
  - Set/get round-trip
  - Unknown key rejection
  - Set on fresh config (only `tree` section exists)
  - JSON output shape

**Design notes:**
- Config file location remains `~/.bo/config.json` (from `config::config_path()`)
- Do not introduce a new config format — stay with the existing JSON structure
- `get` on an unset key should show the effective default (e.g. `gpt-4o` for compile_model)

**Done when:** `bo config set query_model gpt-4.1-mini && bo config get query_model` prints `gpt-4.1-mini`.

---

## 4. README rewrite + tag

**Goal:** A README that lets a stranger install bo, try it, understand what it does, and know its limitations.

**Sections:**

- [ ] **What bo is** — one paragraph. Collect web pages into a local markdown knowledge tree. Query it with citations. No cloud, no vector DB, BYOK.
- [ ] **What bo is not** — not a web scraper, not a search engine, not an autonomous agent. It's a tool for humans and agents to compose.
- [ ] **Install** — `cargo install --path .` (and future crates.io once published)
- [ ] **Quickstart** — seed → collect a URL → list → compile → query. Copy-pasteable commands.
- [ ] **Command reference** — table or list of all commands with one-liner descriptions and key flags
- [ ] **BYOK / provider setup** — OPENAI_API_KEY, `.env` file, `bo config set query_model`
- [ ] **Storage format** — where files live, what frontmatter looks like, what index.jsonl is, what branches are
- [ ] **Limitations + experimental caveat** — lexical retrieval, single provider, no offline/local model yet, tree size bounds, known failure modes
- [ ] **Contributing** — minimal for now (PRs welcome, run CI locally)
- [ ] **License** — MIT

**After README is merged:**

- [ ] `git tag v0.0.1`
- [ ] `git push --tags`
- [ ] Optional: GitHub release with changelog summary

**Done when:** README renders well on GitHub, a new user can go from zero to a successful `bo query` by following it, tag exists.

---

## Sequencing

```
1. Housekeeping  ─┐
                  ├─ can ship independently, no ordering dependency
2. Zero-citation ─┘
3. Config set    ── depends on nothing, but README references it
4. README + tag  ── last (references all prior work)
```

Items 1–3 are parallelizable across sessions. Item 4 is the capstone.
