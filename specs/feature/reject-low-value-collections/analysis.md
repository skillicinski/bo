# Analysis: Reject Low-Value Collections

## Risk assessment

### False positives on valid pages

The main risk is rejecting pages that are useful but contain phrases that look like shell/block/redirect markers.

Examples:

- An article discussing Cloudflare, captchas, or JavaScript requirements.
- A documentation page with UI chrome that includes generic help text.
- A legitimate article with a short body plus footer content.

Mitigation:

- Use conservative multi-signal checks rather than single keyword matches where possible.
- Raw HTML shell/block checks should target strong markers (`cf-mitigated`, `cf-challenge`, `JavaScript is not available`, `http-equiv="refresh"`) rather than generic words alone.
- Extracted-content rejection should require boilerplate dominance, not just a generic title.

### False negatives on low-value pages

Some junk pages will still pass, especially if the site-specific shell text is not represented in fixtures.

Mitigation:

- Accept this for first scope.
- Dogfood after implementation and record remaining misses for later targeted heuristics/adapters.

### Heuristic drift

Sites change shell/error text over time. Heuristics may decay.

Mitigation:

- Keep classifiers deterministic and easy to extend.
- Cover only strongly observed patterns in this feature.
- Treat dogfood corpus as a regression surface.

### Error-shape interaction with existing errors

`main.rs` currently prefixes all command errors with `error: `. The spec requires the rejection body to use `<url> was not collected: <reason>`, but actual stderr will likely be:

```text
error: <url> was not collected: <reason>
```

This is probably acceptable because the collection error itself has the required shape. Tests should avoid over-specifying the CLI prefix unless the CLI is explicitly changed.

### Fetch-level 403 loses body information

`fetch_url` returns `FetchError::HttpStatus` before reading response text. For Medium/Cloudflare, this is enough to classify `403` as blocked, but it means we cannot distinguish all challenge variants at fetch level.

Mitigation:

- Map `401`, `403`, `429` to `BlockedBySite` in `collect_url`.
- Keep raw HTML block detection for `200` challenge pages.

### Artifact atomicity

The plan relies on running all rejection checks before `leaf::write` and `index::append_entry`. If checks are placed too late, a rejection could still leave a partial artifact.

Mitigation:

- Integration tests must assert no markdown files and no index entries for every rejection path.
- Keep all quality gates before slug/write/index.

## Gap analysis

### Definition of "boilerplate-only" is underspecified

The spec names boilerplate/footer-only content but does not define a precise threshold.

Implementation must choose conservative heuristics. Suggested starting point:

- reject extracted bodies that are short and dominated by known footer/shell phrases
- reject OpenReview-style footer-only text that contains project/legal/footer phrases without abstract/content-like substance
- avoid generic length-only rejection beyond existing `EmptyContent`

### Reason precedence is unspecified

A page may match multiple rejection classes, e.g. a JS shell inside a blocked app page.

Suggested precedence:

1. `BlockedBySite`
2. `JsRenderedContent`
3. `RedirectStub`
4. `BoilerplateOnlyContent`

Or, for raw HTML, detect redirect stubs before generic JS if the page is clearly a redirect placeholder. The exact order should be encoded in tests for overlapping fixtures only if necessary.

### `ExtractError::EmptyContent` mapping is unspecified

Empty extraction currently reports `no content extracted`, not `<url> was not collected: boilerplate-only content`.

Decision needed during implementation:

- Leave empty extraction as existing extract error, or map it to collection rejection.

Recommendation:

- Keep existing `EmptyContent` semantics unless tests/spec explicitly require otherwise. This feature is about junk passing as success, not every extraction failure. If mapping is easy and not disruptive, map to `boilerplate-only content`, but avoid broadening scope.

### Title-quality handling is intentionally loose

The plan says title quality may be secondary, but there is no concrete rule.

Recommendation:

- Do not use title quality in first implementation except in tests proving title alone does not reject.
- If later needed, add title heuristics only with dedicated fixtures.

### CLI-level blocked status testing is not explicitly task-scoped

Tasks include mapping HTTP statuses, but tests mostly use `collect_html`. A direct unit test for `classify_http_status` covers status mapping; full `collect_url` status behaviour remains network-dependent unless fetch is refactored or mocked.

Recommendation:

- Unit-test status classification.
- Rely on dogfood/manual network validation for real 403 behaviour.
- Do not refactor fetch just to mock network for this feature.

## Edge cases

- Valid pages with very short but meaningful content may still be rejected by existing extraction length logic; out of scope unless caused by new heuristics.
- Pages with HTTP `200` but embedded captcha/challenge HTML should be rejected by raw HTML block detection.
- Pages with HTTP `403` and useful explanatory HTML should still be classified as blocked; this is acceptable for `bo collect`.
- Redirect stubs may use relative URLs or script patterns not initially detected. First scope should cover observed deterministic patterns only.
- Meta refresh can be used for legitimate timed refresh widgets, but document pages rarely consist only of a refresh stub. Detection should require redirect-like title/body or immediate refresh.
- Pages discussing "please enable JavaScript" in an article could be false positives if detection is phrase-only. Require shell-like surrounding markers where possible.
- OpenReview has useful metadata in static HTML; this feature must not accidentally start relying on metadata extraction, because that is explicitly out of scope.
- Duplicate URL checks should still happen before classification. Re-collecting an existing URL should report duplicate rather than reclassifying the fetched page.
- Failed/rejected URLs should remain retryable because they are not indexed.
- Index absence vs empty index must both count as no artifact.

## Dependencies

### External libraries

No new dependency is required. Adding an HTML parser would increase scope and should be avoided unless string matching proves insufficient for the specified fixtures.

### Network/site behaviour

Dogfood outcomes depend on live site behaviour:

- X may serve `403`, JS shell, or embedded app state depending on headers/location.
- Medium may serve `403` challenge or different block pages.
- OpenReview may change SSR behaviour and extraction result.
- Rust blog redirect stubs may change if the site fixes old URLs.

These should not block implementation because deterministic fixtures define the regression behaviour.

### Existing extractor behaviour

`trafilatura` output shapes determine extracted-content classification. If dependency behaviour changes, tests may need fixture updates.

## Recommendation

Ready to implement with two constraints:

1. Keep first implementation conservative: reject only strong signals represented by fixtures or observed dogfood output.
2. Do not implement recovery, title cleanup, metadata fallback, or source adapters during this feature.

Before coding, decide one small policy point:

- Whether `ExtractError::EmptyContent` remains `no content extracted` or is wrapped as `<url> was not collected: boilerplate-only content`.

Recommended choice: leave existing empty-content errors unchanged for now, unless implementation naturally centralizes extraction failures under the new rejection type without broad side effects.
