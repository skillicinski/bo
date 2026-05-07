# Analysis — YouTube Transcript Adapter

## Risk assessment

### YouTube protocol brittleness

The core implementation depends on an unofficial InnerTube player endpoint and a specific Android client context. It worked during probing, but YouTube can change response shape, require additional request parameters, rate-limit, block client contexts, or return empty caption URLs without warning.

Impact: high. This is the main feature risk.

Mitigation:

- Isolate all InnerTube constants and request/response parsing in the adapter.
- Keep errors explicit and user-facing.
- Add ignored network tests and dogfood validation, but do not make ordinary test runs network-dependent.
- Avoid pretending transcript collection is guaranteed for all public videos.

### Transcript XML format variance

Observed caption XML used `timedtext format="3"` with `<p>` and nested `<s>` nodes. Older examples use `<transcript><text>`. Manual captions may produce different shapes or whitespace behavior.

Impact: medium/high.

Mitigation:

- Parse structurally with XML events rather than brittle regex.
- Unit-test both known shapes.
- Treat empty parsed output as a hard adapter failure.

### Text cleanup correctness

YouTube transcript text can include:

- XML entities: `&amp;`, `&#39;`, `&quot;`
- double-encoded entities: `&amp;#39;`
- `<s>` nodes with leading spaces
- blank `<p>` nodes used for caption layout
- music/sound markers

Impact: medium. Bad cleanup could produce concatenated words, extra spaces, or encoded junk.

Mitigation:

- Preserve intra-segment spacing intentionally.
- Skip empty layout-only paragraphs.
- Decode XML entities and, if necessary, one additional HTML-entity pass.
- Add fixture tests for spacing and entity decoding.

### Collection write-path refactor regression

Refactoring `collect.rs` to share leaf/index writing between HTML and adapter documents could regress ordinary page collection, duplicate detection, slug behavior, or index writes.

Impact: medium.

Mitigation:

- Keep the helper small and behavior-preserving.
- Add/keep ordinary HTML collection tests.
- Avoid changing `leaf`, `slug`, and `index` semantics unless necessary.

### No-write guarantees

The spec requires unsupported or failed YouTube collection to leave no leaf and no index entry. This is easy to violate if file writing occurs before final adapter validation, or if partial writes happen before index append failure.

Impact: medium.

Mitigation:

- Adapter must return a fully valid transcript document before any write helper is called.
- Collect-level tests should assert no markdown files and no `index.jsonl` after failure.

### Async/sync mismatch

The existing collect pipeline is synchronous and uses `reqwest::blocking`. Some YouTube examples/crates are async. Introducing async here would add runtime plumbing to a sync CLI path.

Impact: medium.

Recommendation:

- Use `reqwest::blocking` in the adapter for this MVP.
- Do not introduce async collection unless a broader pipeline refactor is planned.

### Parallel task conflict risk

Many tasks marked `[parallel-safe]` are logically independent but target the same file, `src/adapters/youtube.rs`. Running multiple subagents that edit the same file concurrently will cause merge conflicts.

Impact: medium if using orchestrator/subagents.

Mitigation:

- Either implement pure adapter tasks directly in one pass, or split implementation into submodules before parallel work:
  - `src/adapters/youtube/url.rs`
  - `src/adapters/youtube/transcript.rs`
  - `src/adapters/youtube/innertube.rs`
- If subagents are used, assign exploration/test-design tasks or disjoint files, not concurrent edits to the same file.

## Gap analysis

### URL normalization is underspecified

The spec says the recorded URL is the user-requested YouTube URL in normalized form. It does not define whether normalization should:

- preserve query parameters like `t`, `si`, `feature`, `list`
- drop fragments
- canonicalize `youtu.be/<id>` to `youtube.com/watch?v=<id>`
- treat `youtu.be` and `watch?v=` for the same video as duplicates

Recommended decision for MVP:

- Use `url::Url` normalization and store the normalized user-requested URL exactly as parsed.
- Extract `video_id` for adapter work only.
- Do not canonicalize different URL forms to one video URL.
- Accept that duplicate detection remains exact URL match, matching existing behavior.

### Supported schemes are not explicit

Examples use `https://`, but the existing generic fetch supports `http` and `https`.

Recommended decision:

- Support `http` and `https` for YouTube classification, but normalize/store the parsed URL.
- Reject non-HTTP schemes as invalid/unsupported.

### Extra query parameters on watch URLs

A common URL is `https://www.youtube.com/watch?v=<id>&list=...&t=...`. Playlist collection is out of scope, but the URL still identifies a video.

Recommended decision:

- If path is `/watch` and `v` is valid, collect the video transcript and ignore other query parameters for transcript lookup.
- Preserve the full normalized requested URL in the index.

### Mobile/music/nocookie hosts are unspecified

Not covered:

- `m.youtube.com/watch?v=...`
- `music.youtube.com/watch?v=...`
- `youtube-nocookie.com/embed/...`
- `youtube.com/live/...`
- `youtube.com/clip/...`

Recommended decision:

- Treat these as unsupported YouTube-like URLs for MVP unless exact host support is added deliberately.
- Ensure they fail gracefully rather than falling through to HTML collection when the host is clearly YouTube-owned and unsupported.

### Error taxonomy needs a concrete choice

The plan leaves open whether to wrap adapter errors in `CollectError` or map them to rejection reasons.

Recommended decision:

- Add `CollectError::Youtube(crate::adapters::youtube::YoutubeError)`.
- Keep `RejectReason` for fetched/extracted web-page quality only.
- Display messages should be short: e.g. `unsupported YouTube URL: embed URLs are not collected` or `YouTube transcript unavailable: no English captions found`.

### Transcript quality threshold is undefined

The spec says unavailable/empty transcript fails. It does not say whether a very short transcript like `[Music]` should be rejected as low-value.

Recommended decision:

- MVP rejects only empty/whitespace parsed transcript.
- Do not reuse article quality thresholds unless dogfood shows obvious junk.
- If needed, add a separate future quality task for transcript-specific low-value rejection.

### Dependency choice should be made before implementation

The tasks say decide/add XML dependency. Given the observed XML and entity edge cases, ad-hoc string parsing is likely a false economy.

Recommended decision:

- Add `quick-xml` now.
- Add `html-escape` only if tests prove double-encoded entities survive XML unescaping.

### Network test expectations must stay loose

The supplied URLs had captions during research, but YouTube availability can vary by time, region, IP reputation, or client context.

Recommended decision:

- Keep network test `#[ignore]`.
- Assert broad invariants only: non-empty title/body, no obvious page chrome, no timestamp formatting.
- Do not assert exact transcript text.

## Edge cases

### URL handling

- `https://www.youtube.com/watch?v=<id>&t=30s`
- `https://www.youtube.com/watch?si=x&v=<id>` where `v` is not first
- `https://youtu.be/<id>?si=x&t=30`
- `https://youtube.com/shorts/<id>?feature=share`
- trailing slash after IDs
- empty `v=`
- multiple `v` params
- uppercase/lowercase hosts
- invalid video IDs with spaces, slashes, unicode, or query-only junk
- `youtube.com/embed/<id>` should reject explicitly
- `youtube.com/playlist?list=...` should reject explicitly
- `youtube.com/channel/...`, `/@handle`, `/results?search_query=...` should reject explicitly
- non-YouTube URLs must remain `NotYoutube`
- malformed URLs should surface as invalid URL errors, not panic

### Caption selection

- manual `en` and autogenerated `en` both present: choose manual
- only autogenerated `en`: accept
- only `en-US` or `en-GB`: accept as English
- English translation available but no English source track: do not translate in MVP
- only non-English tracks: fail
- no `captions` object: fail
- caption track missing `baseUrl`: fail/skip safely

### Player response

- `playabilityStatus.status != OK`
- private/deleted/age-restricted/region-blocked video
- video title missing/blank: fall back to video ID
- response is valid JSON but missing expected nested fields
- response is non-JSON error page due to block/rate limit

### Transcript parsing

- empty caption XML with HTTP 200
- valid XML with no text nodes
- malformed XML
- `timedtext` with text directly inside `<p>` and no `<s>` nodes
- nested styling tags inside text
- leading spaces inside `<s>` nodes that are required for word separation
- double-encoded HTML entities
- repeated blank/layout paragraphs

### Collection behavior

- adapter succeeds but URL already exists in index
- adapter fails after network but before write
- leaf write succeeds but index append fails: existing system can still leave a leaf without index; this is not new, but the feature should not worsen it
- same video collected through `youtu.be` and `watch?v=` creates two entries under exact-match duplicate semantics
- slug collision between YouTube title and existing article title

## Dependencies

### External services

- YouTube InnerTube player endpoint availability and response shape.
- YouTube caption `baseUrl` availability.
- Local network/IP reputation/rate limits.
- Regional restrictions or video-specific restrictions.

### Rust dependencies

- `reqwest::blocking` is already available and should be reused.
- `serde`/`serde_json` are already available and sufficient for partial response models.
- `url` is already available and should be used for classification/normalization.
- `quick-xml` is the likely new dependency for robust XML parsing.
- `html-escape` may be needed only for double-encoded transcript entities.

### Test/dogfood dependencies

- Ignored network tests and `./scripts/dogfood-collect video` require live YouTube access.
- These validations may fail due to network environment even if deterministic unit tests pass.

## Recommendation

Ready to implement, with a few decisions locked before code:

1. Use an internal adapter, not a transcript crate.
2. Use synchronous `reqwest::blocking` for adapter HTTP.
3. Add `quick-xml` for transcript parsing.
4. Preserve normalized user-requested URL for storage; do not canonicalize all forms to one watch URL.
5. Exact-match duplicate semantics remain unchanged, even across different URL forms for the same video.
6. Accept `en` and `en-*`; prefer manual over ASR; never translate/fallback to non-English.
7. Add a dedicated YouTube adapter error variant in `CollectError`.
8. If using subagents/orchestrator, first split YouTube code into disjoint submodules or restrict subagents to analysis/test fixture generation to avoid same-file conflicts.

No spec rewrite is required. The implementation should start by creating the adapter structure and parser/URL unit tests before network integration.
