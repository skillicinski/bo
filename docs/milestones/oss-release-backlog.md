# v0.0.1 Release Backlog

Ordered implementation work for the Experimental OSS release. Each section is one session of work. Items map directly to gates in `milestones/oss-release.md`.

---

## 1. Housekeeping

**Goal:** Mechanical release infrastructure. No design decisions.

- [x] Add `LICENSE` file (MIT, full text) to repo root
- [x] Update `Cargo.toml` metadata:
  - `description` — one-line (e.g. "Collect web pages into a local markdown knowledge tree")
  - `repository` — GitHub URL
  - `homepage` — GitHub URL (or separate if exists)
  - `keywords` — e.g. `["cli", "knowledge-base", "markdown", "rag", "web"]`
  - `categories` — e.g. `["command-line-utilities"]`
- [x] Add GitHub Actions CI workflow (`.github/workflows/ci.yml`):
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

- [x] After synthesis, check `cited_slugs` (the validated citation list)
- [x] If `cited_slugs` is empty:
  - Human output: print "no answer from collected sources" (or similar), exit 1
  - JSON output: `{ "status": "error", "error": { "code": "insufficient_sources", "message": "..." } }`
- [x] Add unit tests:
  - Mock synthesis that returns prose with zero valid wikilinks → verify not-answered behavior
  - Mock synthesis that returns prose with ≥1 valid wikilink → verify normal answer output
- [x] Add integration test: query against a tree with irrelevant content for a known-unrelated question → verify exit 1

**Done when:** `bo query "What is the PMNS matrix?"` against the default corpus exits 1 with clear messaging instead of emitting a hallucinated answer.

---

## 3. `bo config set` MVP

**Goal:** Users can change the model without hand-editing JSON.

**Design decision (2026-05-11, revised during implementation):** Single `model` field for v0.0.1. One knob, affects all LLM stages (compile, query, summary). Per-stage overrides (`compile_model`, `query_model`) are not retained for the unpublished pre-release config shape.

Fallback hierarchy: `model` > `"gpt-4o"`.

**Implementation:**

- [x] Add `model: Option<String>` to `Config` struct
- [x] Update LLM-backed commands to use `model` with `"gpt-4o"` default
- [x] Add `Config` subcommand to CLI:
  ```
  bo config set model <value>
  bo config get model
  ```
- [x] `set` reads existing config, updates the field, writes back
- [x] `get` prints the current effective value
- [x] Unknown key → exit 2 with error listing valid keys
- [x] `--json` support on both subcommands
- [x] Unit tests:
  - Set/get round-trip
  - Unknown key rejection
  - `model` > default fallback
  - JSON output shape

**Done when:** `bo config set model gpt-4.1-mini && bo config get model` prints `gpt-4.1-mini`.

---

## 4. Clean installation happy path

**Goal:** A new user can install bo from GitHub and run `bo seed` from any local shell without cloning the repo for contribution work.

**Why this is a release gate:** The README quickstart is only credible if the install command produces a working `bo` binary outside the repository checkout. Full packaging can wait, but the GitHub/cargo path must be real.

**Scope:** Separate feature branch from auth configuration. This is not crates.io/Homebrew/binary distribution; those remain post-v0.0.1 packaging work.

**Implementation:**

- [ ] Document the release install command:
  ```bash
  cargo install --git https://github.com/jellysnake/bo --tag v0.0.1
  ```
- [ ] Keep the contributor/local-dev install command distinct:
  ```bash
  cargo install --path .
  ```
- [ ] Add an install smoke check or release checklist that validates an installed binary from outside the repo:
  - `bo --help`
  - `bo config get model`
  - `bo seed <temp-tree>`
  - `bo list`
- [ ] Run the smoke check with a temporary `HOME` and arbitrary current working directory.
- [ ] Verify installed bo does not require repo-relative files, `tmp-tree`, or a checked-out `.env`.
- [ ] If using CI for this gate, approximate tag install with local install (`cargo install --path . --locked --root <tmp-root>`) and manually verify the `--git --tag v0.0.1` path after tagging.

**Done when:** From a clean shell with Rust installed, a user can run the documented install command, then `bo seed ~/bo-tree`, without being inside the bo repo.

---

## 5. Auth configuration happy path

**Goal:** Users can configure their OpenAI API key through bo itself instead of relying on `.env`, shell exports, or cloning the repo.

**Why this is a release gate:** Auth setup is one of the first three actions for a test user. Requiring `source .env` makes the public quickstart feel like an internal dogfood workflow.

**Scope:** Separate feature branch from clean installation. OpenAI only for v0.0.1. This is local secret storage, not full OS keychain integration.

**User-facing command:**

```bash
bo config auth --provider openai
```

**Behavior:**

- [ ] Prompts for the OpenAI API key without echoing input.
- [ ] Creates `~/.bo/auth.json` if absent.
- [ ] Stores OpenAI auth separately from `~/.bo/config.json`.
- [ ] Uses a shape like:
  ```json
  {
    "providers": {
      "openai": {
        "api_key": "sk-..."
      }
    }
  }
  ```
- [ ] Applies restrictive file permissions where supported (`0600` on Unix/macOS).
- [ ] Running the command again overwrites the stored OpenAI key.
- [ ] Never prints the key in human output, JSON output, logs, or errors.
- [ ] Unknown provider exits 2 and lists valid providers (`openai`).
- [ ] Supports global `--json`; JSON success reports provider/status but never the secret.

**API key resolution:**

- [ ] LLM-backed commands resolve API keys in this order:
  1. `OPENAI_API_KEY` environment variable
  2. `~/.bo/auth.json`
  3. clear setup error
- [ ] `OPENAI_API_KEY` remains supported as an override for CI, temporary sessions, and advanced users.
- [ ] Compile/query missing-key errors point to `bo config auth --provider openai`.
- [ ] Summary generation uses the same resolver while preserving deterministic fallback when no key is configured.

**Tests:**

- [ ] Unit tests for auth read/write round-trip.
- [ ] Unit tests for overwrite behavior.
- [ ] Unit tests for malformed auth file behavior.
- [ ] Unit or integration test that stored auth is used when `OPENAI_API_KEY` is absent.
- [ ] Integration test for unknown provider exit 2.
- [ ] JSON output tests for success/error shape with no leaked key.
- [ ] Unix/macOS test or best-effort assertion for restrictive file permissions.

**Done when:** A fresh user can install bo, run `bo config auth --provider openai`, then run LLM-backed bo commands without `.env` or `source .env`.

---

## 6. README rewrite + tag

**Goal:** A README that lets a stranger install bo, authenticate, try it, understand what it does, and know its limitations.

**Sections:**

- [ ] **What bo is** — one paragraph. Collect web pages into a local markdown knowledge tree. Query it with citations. No cloud, no vector DB, BYOK.
- [ ] **What bo is not** — not a web scraper, not a search engine, not an autonomous agent. It's a tool for humans and agents to compose.
- [ ] **Install** — `cargo install --git https://github.com/jellysnake/bo --tag v0.0.1`; contributor path `cargo install --path .`; future crates.io once published.
- [ ] **Quickstart** — install → `bo config auth --provider openai` → seed → collect a URL → list → compile → query. Copy-pasteable commands.
- [ ] **Command reference** — table or list of all commands with one-liner descriptions and key flags.
- [ ] **BYOK / provider setup** — `bo config auth --provider openai`, `OPENAI_API_KEY` override, `bo config set model`.
- [ ] **Storage format** — where files live, what frontmatter looks like, what index.jsonl is, what branches are, where config/auth files live.
- [ ] **Limitations + experimental caveat** — lexical retrieval, single provider, no offline/local model yet, tree size bounds, known failure modes.
- [ ] **Contributing** — minimal for now (PRs welcome, run CI locally).
- [ ] **License** — MIT.

**After README is merged:**

- [ ] `git tag v0.0.1`
- [ ] `git push --tags`
- [ ] Verify documented `cargo install --git https://github.com/jellysnake/bo --tag v0.0.1` works from a clean shell
- [ ] Optional: GitHub release with changelog summary

**Done when:** README renders well on GitHub, a new user can go from zero to a successful `bo query` by following it, tag exists, and the tag install command works.

---

## Sequencing

```
1. Housekeeping  ─┐
                  ├─ done
2. Zero-citation ─┘
3. Config set    ── done; README references it
4. Clean install ── separate release-gate branch; README depends on it
5. Auth config   ── separate release-gate branch; README depends on it
6. README + tag  ── capstone; references all prior work
```

Items 4 and 5 are independent feature branches and can be done in either order. Item 6 is last.
