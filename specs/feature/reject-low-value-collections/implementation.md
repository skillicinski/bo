# Implementation: Reject Low-Value Collections

## Summary

Implemented a conservative quality gate in the `bo collect` pipeline so clearly low-value/non-document pages are rejected before leaf or index artifacts are written.

Rejected collections now return a user-facing error in this shape:

```text
<url> was not collected: <reason>
```

Supported rejection reasons:

- `blocked by site`
- `JS-rendered content`
- `redirect stub`
- `boilerplate-only content`

## What changed

### Added quality classification

Added `src/quality.rs` as a private module with deterministic classifiers for:

- HTTP status rejection:
  - `401`, `403`, `429` → `blocked by site`
- Raw HTML rejection:
  - Cloudflare/captcha/access-challenge shells
  - JavaScript-required/app shells
  - deterministic redirect stubs
- Extracted-content rejection:
  - redirect stub output
  - JS-required shell output
  - blocked/challenge output
  - boilerplate/footer-only output, including the observed OpenReview shell/footer case

`RejectReason` is re-exported from `src/lib.rs`; classifier internals remain private to the crate.

### Integrated into collection pipeline

Updated `src/collect.rs`:

- Added `CollectError::Rejected { url, reason }`.
- Rendered rejection errors as `<url> was not collected: <reason>`.
- Mapped blocked HTTP status fetch failures in `collect_url` to rejections.
- Ran raw HTML classification in `collect_html` before extraction.
- Ran extracted-content classification after extraction but before slug/write/index.
- Preserved existing `ExtractError::EmptyContent` behaviour.

The artifact-safety invariant is now:

```text
reject before leaf::write and before index::append_entry
```

### Tests added

Updated `tests/integration.rs` with behaviour tests for:

- redirect stub rejection with no artifacts
- X/Twitter-style JavaScript shell rejection with no artifacts
- OpenReview footer-only rejection with no artifacts
- Cloudflare/block shell rejection with no artifacts
- mdBook/Rust Book-like page acceptance despite bad UI title

Added classifier unit tests in `src/quality.rs` for:

- blocked status mapping
- redirect stub detection
- JavaScript shell detection
- block/challenge detection
- OpenReview footer-only extracted content
- false-positive protection for valid articles that merely mention suspicious terms
- false-positive protection for bundled `window.location.href` JavaScript in valid article pages

### Other cleanup

Updated `tests/integration_compile.rs` for clippy compliance:

- replaced `map_or(false, ...)` with `is_some_and(...)`
- replaced `filter(...).next()` with `find(...)`

## Dogfood results

Ran:

```bash
scripts/dogfood-collect default
```

Final result:

```text
39 ok, 3 failed
```

Expected explicit rejections:

- `https://blog.rust-lang.org/2015/05/11/traits.html`
  - `redirect stub`
- `https://openreview.net/forum?id=OAudWSf7aH`
  - `boilerplate-only content`
- `https://x.com/lifeof_jer/status/2048103471019434248`
  - `JS-rendered content`

Known-good pages continued collecting.

Rust Book/mdBook pages still collect, even though title extraction remains poor (`Keyboard shortcuts`), because the body content is useful. Title repair remains a separate feature.

Medium behaviour was nondeterministic:

- earlier runs saw `403`/Cloudflare-style blocking
- latest `bo collect` dogfood collected the article successfully
- direct `curl -A bo/0.1` still received Cloudflare `403`

Conclusion: Medium behaviour appears to vary by client/fingerprint/timing/edge. This implementation handles both outcomes:

- blocked response → reject as `blocked by site`
- real article response → collect normally

## Validation

Passed:

```bash
cargo fmt
cargo test
cargo clippy --all-targets -- -D warnings
```

Also passed dogfood validation with expected explicit rejections.

## Deliberately out of scope

This implementation did not add:

- redirect following
- source-specific recovery adapters
- X/Twitter via xcancel
- YouTube transcript collection
- OpenReview metadata/API extraction
- Medium bypassing or anti-bot circumvention
- title cleanup/fallback logic
- `--force` override mode

## Follow-up candidates

Captured separately in `scratchpad/features.md`:

- attempt X/Twitter collection through xcancel
- add YouTube transcript URL adapter
- improve extracted titles when UI chrome pollutes document metadata

Additional likely follow-up:

- investigate Medium/Cloudflare client variance between `reqwest` and `curl`
