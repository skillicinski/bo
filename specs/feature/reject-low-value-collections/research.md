# Research: Reject Low-Value Collections

## Technical unknowns to validate

### Heuristic precision

Need to validate that rejection heuristics catch the observed bad cases without rejecting useful pages.

Observed bad cases:

- X/Twitter JS-disabled/app shell
- Medium/Cloudflare access challenge
- Rust blog deterministic redirect stub
- OpenReview shell/footer-only extraction

Validation approach:

- Use small fixtures for deterministic tests.
- Re-run `scripts/dogfood-collect default` after implementation.
- Inspect any newly rejected default-corpus URLs for false positives.

### OpenReview extraction behaviour

OpenReview pages include useful metadata in HTML tags, but generic body extraction can yield footer/shell content.

For this feature:

- reject footer-only extraction as low-value
- do not implement metadata fallback

Future feature candidate:

- source-specific OpenReview adapter or metadata fallback.

### Title quality interaction

mdBook/Rust Book pages can have a wrong extracted title (`Keyboard shortcuts`) while retaining useful body content.

For this feature:

- do not reject solely on title quality
- only use bad/generic titles as secondary evidence when body is low-value

Future feature candidate:

- title selection cleanup using document `<title>` or main heading.

## Libraries or APIs to evaluate

No new library/API required for first implementation.

Potential future options, out of scope now:

- HTML parser for more precise meta refresh/script redirect detection
- site adapters for X/Twitter, OpenReview, YouTube, Medium-like publishers

## Performance considerations

Classification is cheap string scanning over already-fetched HTML and extracted markdown.

Expected impact:

- negligible CPU overhead compared to network fetch and trafilatura extraction
- no extra network calls
- no browser execution

## Operational considerations

False positives are more harmful than missing some low-value pages because rejected URLs are not recorded and users may assume collection is unsupported. Initial heuristics should therefore be conservative and based on strong signals.

False negatives are acceptable for first scope if they can be observed through dogfood and tightened later.
