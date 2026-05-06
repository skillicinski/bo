# Feature Scratchpad

Somewhat formulated feature candidates. Keep items tickable. Promote one item at a time to `specs/<branch>/` with `/speckit.specify <branch>` unless there is a strong reason to bundle scope.

## Candidates

- [x] Reject suspicious low-value collections instead of silently writing junk
  - Context: dogfood showed `bo collect` accepts pages when `HTTP 2xx + HTML + extracted body >50 chars`, even when the body is a redirect stub, JS-disabled shell, app footer, or boilerplate.
  - Expected: suspicious/low-value pages fail explicitly with a useful reason and do not write a leaf or index entry.
  - Notes: first scope should classify/reject; source-specific recovery/adapters should be separate specs.

- [ ] Attempt X/Twitter collection through an xcancel adapter
  - Context: direct X/Twitter fetches can return JS-required/error shell instead of tweet content.
  - Expected: for supported `x.com` / `twitter.com` status URLs, try an `xcancel.com` adapter path and collect useful tweet content if available.
  - Notes: `https://xcancel.com/about`; keep fallback/failure semantics explicit.

- [ ] Add a YouTube transcript URL adapter
  - Context: YouTube videos are not ordinary article pages, but transcripts are often the useful collectable text.
  - Expected: for supported YouTube video URLs, fetch transcript text through open/public APIs if viable, or fail explicitly if transcripts are unavailable/disabled.
  - Notes: viability check should be part of the spec/research phase.

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
