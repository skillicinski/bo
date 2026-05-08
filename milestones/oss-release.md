# OSS Release Milestone

Path from dogfood alpha to public open-source release.

Credible dogfood alpha today; not yet stable public software. Target loop:

```text
collect → compile → query/read/navigate → refine tree → repeat
```

Current state is closer to `collect → compile → files exist`.

Core storage philosophy holds: markdown tree is user-owned and inspectable; any DB/index/snapshot layer is acceleration, not source of truth.

## Readiness levels

**Experimental OSS** (~1–2 sessions):
- basic README expansion
- at least one inspect command: `list` or `status`
- basic scan/survey or rebuild-index
- CI basics
- explicit experimental caveat

**Useful alpha** (~3–5 sessions):
- `bo query` MVP
- deterministic local search
- tree health scan/survey
- compile dry-run or snapshot
- better docs/examples

**Public beta** (~6–10 sessions):
- snapshots/history
- query with citations
- robust compile recovery
- dogfood expectations
- release packaging
- source-adapter hardening

## Candidate implementation ideas

1. Add deterministic `bo list` for leaves/branches.
2. Add `bo show <slug>` for rendering one leaf/branch in terminal.
3. Add deterministic lexical `bo search <term>`.
4. Add `bo status` with tree summary and last compile state.
5. Add `bo query` MVP using lexical retrieval and no mutation.
6. Add provider-backed `bo query` answer synthesis with citations.
7. Add model/provider config for BYOK query/compile providers.
8. Add local/OpenAI-compatible LLM endpoint support.
9. Add `bo survey` / `bo scan` for tree health diagnostics.
10. Add `bo rebuild-index` from leaf frontmatter.
11. Add `bo prune <slug-or-id>` for explicit managed deletion.
12. Add snapshot manifest MVP before compile/mutations.
13. Add compile dry-run with planned branch/write preview.
14. Add compile final validation gate before writes.
15. Add dogfood regression expectations for corpora.
16. Add X/Twitter adapter through xcancel.
17. Add PDF URL collection.
18. Add RSS feed collection.
19. Add local/remote markdown file collection.
20. Add podcast/audio transcript collection research spike.
21. Improve YouTube transcript paragraph grouping.
22. Add optional YouTube timestamp/source links.
23. Improve title extraction for documentation/UI-chrome pages.
24. Add CI and dependency/security checks.
25. Write v0.0.1 README/release docs.

## Recommended sequence

1. `bo status` or `bo list` — quick deterministic user confidence.
2. `bo survey` / scan — tree health and repair visibility.
3. `bo search` — deterministic retrieval groundwork.
4. `bo query` lexical MVP — completes the core loop.
5. compile safety/snapshot work — protects generated writes.

Avoid jumping to DB-backed query until the deterministic inspection/search layer exists.
