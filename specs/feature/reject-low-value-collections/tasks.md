# Tasks: Reject Low-Value Collections

## Setup / baseline

- [x] Confirm working branch is `feature/reject-low-value-collections` and note any unrelated uncommitted files.
- [x] Run baseline `cargo test` and record any pre-existing failures.
- [x] Add or identify a reusable test helper for asserting rejected collection attempts leave no markdown files and no index entries.

## Quality classifier

- [x] Add `src/quality.rs` with `RejectReason` variants and user-facing `Display` strings.
- [x] Add `classify_http_status(status: u16)` mapping `401`, `403`, and `429` to `BlockedBySite`.
- [x] Add raw HTML redirect-stub detection for deterministic meta/script redirect placeholders.
- [x] Add raw HTML JS-required/app-shell detection for pages that require JavaScript and expose no static document content.
- [x] Add raw HTML block/challenge detection for captcha, Cloudflare, access-denied, and publisher-block pages.
- [x] Add extracted-content boilerplate/footer-only detection, including an OpenReview-style shell/footer fixture.
- [x] Add classifier tests proving generic or wrong titles do not cause rejection when body content is substantive.
- [x] Add classifier tests proving valid article text that merely mentions JavaScript, captcha, redirect, or Cloudflare terms is not rejected by single-keyword matching.

## Collect integration

- [x] Export the `quality` module from `src/lib.rs`.
- [x] Extend `CollectError` with a rejection variant containing URL and `RejectReason`.
- [x] Render collection rejection errors as `<url> was not collected: <reason>`.
- [x] Map blocked HTTP status fetch errors in `collect_url` to collection rejections.
- [x] Preserve existing `ExtractError::EmptyContent` behaviour unless implementation naturally wraps it without broad side effects.
- [x] Run raw HTML classification in `collect_html` before extraction and return rejection before any write.
- [x] Run extracted-content classification in `collect_html` before slug/write/index and return rejection before any write.

## Integration tests

- [x] Add a redirect-stub fixture test proving collection is rejected and no artifacts are written.
- [x] Add an X/Twitter-style JS shell fixture test proving collection is rejected and no artifacts are written.
- [x] Add an OpenReview/footer-only fixture test proving collection is rejected and no artifacts are written.
- [x] Add a Cloudflare/block-style fixture test proving collection is rejected and no artifacts are written.
- [x] Add an mdBook-ish fixture test proving substantive body content is accepted even when UI chrome produces a bad/generic title.
- [x] Confirm the existing normal article happy-path integration test still passes unchanged.
- [x] If adding CLI-level assertions, allow the existing `error: ` prefix and assert the `<url> was not collected: <reason>` body.

## Validation

- [x] Run `cargo fmt`.
- [x] Run `cargo test` and fix any failures introduced by the feature.
- [x] Run `scripts/dogfood-collect default`.
- [x] Inspect dogfood results and verify suspicious URLs now fail explicitly while known-good articles/documents still collect.
- [x] Update scratchpad notes with dogfood findings if any unexpected false positive or false negative remains.

## Parallelization notes

After baseline, the following work can be parallelized:

- Quality classifier implementation/tests can be done mostly independently of collect integration once `RejectReason` shape is agreed.
- Integration fixture tests can be drafted independently against the intended public behaviour, then wired once collect integration exists.
- Dogfood validation must run after implementation and unit/integration tests pass.

Safe parallel groups:

1. `quality` module + unit tests.
2. `collect` error/integration wiring.
3. Integration fixture tests and no-artifact helper.

Sequential dependencies:

- Exporting/using `quality` depends on the module shape.
- Dogfood depends on all implementation tasks.
- Final scratchpad update depends on dogfood findings.
