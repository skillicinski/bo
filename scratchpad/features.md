# Feature Scratchpad

Somewhat formulated feature candidates. Keep items tickable. Promote one item at a time to `specs/<branch>/` with `/speckit.specify <branch>` unless there is a strong reason to bundle scope.

## Candidates

- [x] Reject suspicious low-value collections instead of silently writing junk
  - Context: dogfood showed `bo collect` accepts pages when `HTTP 2xx + HTML + extracted body >50 chars`, even when the body is a redirect stub, JS-disabled shell, app footer, or boilerplate.
  - Expected: suspicious/low-value pages fail explicitly with a useful reason and do not write a leaf or index entry.
  - Notes: first scope should classify/reject; source-specific recovery/adapters should be separate specs.

- [ ] Add deterministic tree inspection commands
  - Context: users and agents need to see what is in a tree without manually browsing markdown files.
  - Expected: add low-risk commands such as `bo list`, `bo show <slug>`, and `bo status` for leaves/branches, recent collection state, and compile metadata.
  - Notes: no network or LLM dependency; useful before query.

- [ ] Add deterministic lexical search
  - Context: query needs a deterministic retrieval foundation before LLM answer synthesis.
  - Expected: `bo search <term>` returns matching leaves/branches with file, title, URL, and short context snippets.
  - Notes: start with simple lexical ranking/BM25-style scoring; no mutation.

- [ ] Add lexical `bo query` MVP with citations
  - Context: the core product loop is incomplete until users can ask questions over the collected tree.
  - Expected: `bo query <question>` retrieves local evidence lexically and returns an answer with cited files/URLs, without modifying the tree.
  - Notes: LLM synthesis can be a follow-up; first scope should make retrieval/citation behaviour testable.

- [ ] Add BYOK LLM answer synthesis for query
  - Context: after deterministic retrieval works, users should be able to bring provider API keys for answer synthesis over retrieved context.
  - Expected: support OpenAI, Anthropic, and OpenRouter first; answer with citations from retrieved tree content.
  - Notes: no backend/SaaS requirement; keep provider config shared with compile where possible.

- [ ] Add local/OpenAI-compatible LLM endpoint support
  - Context: users may want local or self-hosted inference through llama.cpp, vLLM, Ollama, or other OpenAI-compatible servers.
  - Expected: configure a base URL/model/key for local query/compile use without changing command semantics.
  - Notes: keep separate from initial BYOK hosted-provider support unless trivial.

- [ ] Add tree health survey/scan
  - Context: users/LLMs can break markdown, frontmatter, references, or indices; bo needs a deterministic way to report tree health.
  - Expected: `bo survey` or `bo scan` reports bad YAML, empty files, missing index entries, index entries pointing to missing files, duplicate URLs, orphan files, and broken branch references.
  - Notes: read-only first; repair commands should be explicit.

- [ ] Add index rebuild from leaf frontmatter
  - Context: `index.jsonl` is a derived cache and may be deleted or corrupted.
  - Expected: `bo rebuild-index` reconstructs index entries from managed leaf frontmatter and reports conflicts or invalid files.
  - Notes: should pair well with survey/scan diagnostics.

- [ ] Add explicit prune command for managed tree entries
  - Context: users need a safe way to remove dead leaves/branches one at a time after survey/scan identifies issues.
  - Expected: `bo prune <slug-or-id>` removes the selected managed leaf/branch and updates derived indices/references as appropriate.
  - Notes: destructive operation; require exact target and clear output.

- [ ] Add snapshot manifest MVP for tree state safety
  - Context: user edits, LLM compile runs, or partial failures can damage tree state.
  - Expected: capture snapshot metadata before risky mutations: file paths, hashes, frontmatter summaries, index state, branch state, and config version.
  - Notes: keep markdown tree authoritative; consider git/git-like storage or manifests under `~/.bo` to reduce blast radius.

- [ ] Add compile dry-run and planned write preview
  - Context: `bo compile` invokes an agent and writes generated branches/frontmatter, so users need to inspect planned changes first.
  - Expected: `bo compile --dry-run` traverses/plans without writing and prints or stores the proposed writes/diffs.
  - Notes: should use deterministic tools for tree reads and validation.

- [ ] Add final validation gate before compile writes
  - Context: successful agentic systems often gate planned writes with deterministic checks and/or a focused reviewer before mutation.
  - Expected: validate branch/frontmatter shape, referenced leaf existence, non-empty outputs, no invented files, and sane diff size before applying compile writes.
  - Notes: can later use a small/specialized LLM as an additional review gate, but deterministic validation should come first.

- [ ] Add collection/rejection event ledger and retry command
  - Context: failed collections are currently transient CLI output, making it hard to audit or retry failures later.
  - Expected: persist collection/rejection/duplicate/fetch-failed events and add `bo retry` / `bo retry-rejected` for selected failures.
  - Notes: lower priority than query/inspection because manual retries are cheap today.

- [ ] Add CI and dependency/security checks
  - Context: open-source readiness needs deterministic checks outside a local session.
  - Expected: GitHub Actions for `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test`, and dependency/security checks such as `cargo deny`.
  - Notes: do near release-readiness work, before comprehensive README.

- [ ] Write v0.0.1 README and release docs
  - Context: current README is too thin for external users.
  - Expected: document what bo is/is not, install, quickstart, command reference, storage format, privacy/security model, BYOK providers, limitations, dogfood, and ignored tests.
  - Notes: dedicate a separate session when commands are close to first release shape.

- [ ] Attempt X/Twitter collection through an xcancel adapter
  - Context: direct X/Twitter fetches can return JS-required/error shell instead of tweet content.
  - Expected: for supported `x.com` / `twitter.com` status URLs, try an `xcancel.com` adapter path and collect useful tweet content if available.
  - Notes: `https://xcancel.com/about`; keep fallback/failure semantics explicit.

- [x] Add a YouTube transcript URL adapter
  - Context: YouTube videos are not ordinary article pages, but transcripts are often the useful collectable text.
  - Expected: for supported YouTube video URLs, fetch plain transcript text through open/public APIs/a crate if viable, title the leaf from the YouTube video page title, or fail explicitly if transcripts are unavailable/disabled.
  - Notes: implemented as an internal InnerTube adapter; MVP omits timestamps and references.

- [ ] Improve YouTube transcript adapter output and dependency posture
  - Context: transcript text is more useful to readers/LLMs when passages can be confirmed against the source video, but the MVP should stay plain and small. Dogfood also showed autogenerated transcript cue chunking can create hundreds of very short paragraphs, which is readable but not ideal prose.
  - Expected: optionally include section/paragraph-level timestamps or source links, improve cue grouping into more natural paragraphs, and keep the internal InnerTube/XML implementation small and observable.
  - Notes: keep separate from adapter MVP unless the current output becomes unreadable.

- [ ] Add PDF URL collection adapter
  - Context: useful documents are often PDFs linked by URL, but current fetch rejects non-HTML content.
  - Expected: for PDF URLs, extract readable text into a normal markdown leaf or fail explicitly when text extraction is unavailable/empty.
  - Notes: research dependency and extraction quality first; scanned/OCR PDFs can be out of scope.

- [ ] Add RSS feed collection
  - Context: feeds are useful source lists and can seed repeated collection workflows.
  - Expected: collect RSS/Atom feed URLs into feed metadata and/or collect feed item links deterministically with clear duplicate behaviour.
  - Notes: decide whether MVP writes a feed leaf, enqueues item URLs, or both.

- [ ] Add local and remote markdown file collection
  - Context: users may already have markdown files locally or hosted over URL that should enter the tree without HTML extraction.
  - Expected: support collecting `.md` content from local paths and markdown URLs into leaves with frontmatter/index entries.
  - Notes: preserve source path/URL; avoid rewriting markdown unnecessarily.

- [ ] Research podcast/audio transcript adapters
  - Context: podcasts and talks on Spotify, iHeartRadio, SiriusXM, Wondery, and generic audio feeds can contain useful collectable text, but transcript availability varies.
  - Expected: identify viable public transcript paths or ASR workflow boundaries before implementation.
  - Notes: research spike first; do not bundle provider-specific audio adapters.

- [ ] Improve extracted titles when UI chrome pollutes document metadata
  - Context: mdBook/Rust Book pages currently collect useful body content but title/frontmatter/slug can become `Keyboard shortcuts` from page chrome.
  - Expected: prefer content-specific headings/titles over navigation/help/UI labels, e.g. Rust Book ownership page becomes `Understanding Ownership`.
  - Notes: keep separate from low-value rejection; this is extraction quality, not collection acceptance.

- [ ] Add dogfood regression expectations for corpus URLs
  - Context: dogfood caught both bad collections and a false positive during low-value rejection work, but result inspection is currently manual.
  - Expected: encode expected `ok`/rejected categories for selected corpus URLs so regressions are visible without manually reading every leaf.
  - Notes: keep network variability in mind; likely support loose expectations or a smaller stable regression corpus.

- [ ] Investigate Medium/Cloudflare client variance
  - Context: Medium sometimes blocks `bo collect`/reqwest and sometimes serves the article, while `curl -A bo/0.1` still receives Cloudflare `403`.
  - Expected: identify whether variance is due to headers, HTTP/2/TLS fingerprinting, cookies, region/edge, timing, or bot score.
  - Notes: research spike first; do not implement bypass/circumvention unless separately specified.

## Promotion rule

Default: promote exactly one candidate into Speckit at a time.

1. Pick one checked/selected candidate.
2. Run `/speckit.specify <short-branch-slug>`.
3. Convert the item context/expected/notes into a user-facing spec.
4. Keep bundled work out unless it is required for the selected candidate.
