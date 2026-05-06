# Implementation Plan: Reject Low-Value Collections

## Architecture decisions and rationale

### Add a collection quality gate

Introduce a quality/classification step in the `bo collect` pipeline before any leaf or index entry is written.

Current pipeline:

```text
fetch URL → extract content → write leaf → append index
```

Planned pipeline:

```text
fetch URL
→ classify raw fetch/HTML result
→ extract readable content
→ classify extracted content
→ write leaf
→ append index
```

Rationale:

- The existing success condition (`HTTP 2xx + HTML + extracted body >50 chars`) is too weak.
- Bad pages can pass extraction and become trusted leaves.
- Classification before write preserves the all-or-nothing behaviour users expect from collection.

### Fail closed only for strong signals

The quality gate should reject obvious non-documents, not attempt broad content scoring. Prefer conservative multi-signal checks for generic terms so valid articles that merely discuss JavaScript, captchas, redirects, or Cloudflare are not rejected.

Examples of strong signals:

- access/block/challenge statuses (`401`, `403`, `429`)
- JavaScript-required shell text
- anti-bot/captcha/challenge page text or metadata
- deterministic HTML/JS redirect stubs
- extracted boilerplate/footer-only text with no substantive document body

Rationale:

- Avoid false positives on unusual but valid pages.
- Do not reject pages solely for a bad title.
- Keep recovery/adapters out of this feature.

### Keep rejection reasons explicit and user-facing

Add a first-class rejection error to the collection pipeline. The CLI should surface messages in this shape:

```text
<url> was not collected: <reason>
```

Reason examples:

- `blocked by site`
- `JS-rendered content`
- `redirect stub`
- `boilerplate-only content`

Rationale:

- Users need to know whether a URL is blocked, unsupported, or simply not an article-like document.
- These categories guide later feature work without promising recovery now.

### Do not solve title cleanup here

A page with real body content must not be rejected solely because its title is generic, wrong, or UI-derived.

Rationale:

- Rust book/mdBook pages demonstrate that title extraction can be wrong while body extraction is useful.
- Title cleanup is a separate feature.
- Title quality may only be used as a secondary signal when the body is also low-value.

## Key components and responsibilities

### `quality` module

New module responsible for classifying raw HTML, extracted content, and fetch-level conditions.

Responsibilities:

- define rejection reason categories
- detect status-level blocking conditions
- detect raw HTML non-document shells/stubs
- detect extracted-content boilerplate/low-value output
- expose small deterministic functions usable from tests

Likely public API:

```rust
pub enum RejectReason {
    BlockedBySite,
    JsRenderedContent,
    RedirectStub,
    BoilerplateOnlyContent,
}

pub fn classify_http_status(status: u16) -> Option<RejectReason>;
pub fn classify_html(html: &str) -> Option<RejectReason>;
pub fn classify_extracted(title: Option<&str>, body_markdown: &str) -> Option<RejectReason>;
```

Exact naming can change during implementation.

### `collect` module

Integrate quality gates into `collect_url` and `collect_html`.

Responsibilities:

- preserve duplicate detection before expensive work
- run raw HTML classification before extraction
- run extracted-content classification before slug/write/index
- return a rejection error before any artifact is written

### `fetch` module

Keep fetch mostly unchanged.

Potential minimal adjustment:

- map `FetchError::HttpStatus(401 | 403 | 429, _)` to a collection rejection in `collect_url`, so blocked URLs get the user-facing rejection shape.

Avoid changing fetch semantics unless needed.

### CLI

No new flags.

The existing `cmd_collect` error path can remain if `CollectError::Display` produces the required message. Since `main.rs` prefixes command failures with `error: `, CLI stderr may be `error: <url> was not collected: <reason>`; tests should assert the collection-error body rather than require removal of the existing prefix.

## Integration points and external dependencies

### Existing dependencies

- `reqwest`: fetches HTML and exposes status errors.
- `trafilatura`: extracts readable markdown and metadata.

### New dependencies

No new external dependencies planned.

Detection should use deterministic string/HTML-pattern heuristics first. If parsing becomes necessary, evaluate whether existing dependency transitive crates are available, but prefer avoiding dependency expansion for this feature.

## Implementation strategy

1. Add a `quality` module with `RejectReason` and deterministic classifiers.
2. Add unit tests for quality classifiers using small fixtures:
   - JS-required shell
   - Cloudflare/captcha/block shell
   - meta/script redirect stub
   - OpenReview-style footer-only extracted content
   - generic-title page with substantive body accepted
3. Extend `CollectError` with a rejection variant containing URL and reason.
4. Integrate raw HTML classification in `collect_html` before extraction.
5. Integrate extracted-content classification in `collect_html` before slug/write/index.
6. Map blocked HTTP statuses in `collect_url` to rejection errors.
7. Leave existing `ExtractError::EmptyContent` behaviour unchanged unless the implementation naturally maps it to a rejection without broad side effects.
8. Add integration tests proving rejected URLs leave no markdown file and no index entry.
9. Re-run default dogfood and verify:
   - known-good pages still collect
   - suspicious pages fail explicitly instead of writing junk

## Test strategy

### Unit tests

Focus on deterministic classifier behaviour in the new module.

### Integration tests

Use inline HTML fixtures through `bo::collect::collect_html` to avoid network dependency.

Required scenarios:

- redirect stub rejected with no artifacts
- JS-required shell rejected with no artifacts
- boilerplate/footer-only extraction rejected with no artifacts
- generic bad title with real body is accepted
- normal article remains accepted

### Network dogfood

Manual dogfood after implementation:

```bash
scripts/dogfood-collect default
```

Expected changes:

- X/Twitter direct URL should fail explicitly.
- OpenReview shell/footer result should fail explicitly unless extraction now yields enough real content.
- Rust redirect stub should fail explicitly.
- Medium should fail explicitly as blocked.
- Known-good corpus items should continue collecting.
