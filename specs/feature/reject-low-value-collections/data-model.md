# Data Model: Reject Low-Value Collections

## Core entities

### Rejection reason

Represents why a URL was fetched or extracted but not accepted as a document.

Fields/variants:

- `BlockedBySite`
  - The site denied access, served an access challenge, or otherwise blocked collection.
- `JsRenderedContent`
  - The fetched page requires JavaScript/app execution and did not expose useful document content as static HTML.
- `RedirectStub`
  - The fetched page is a deterministic redirect placeholder rather than document content.
- `BoilerplateOnlyContent`
  - Extraction produced only shell/footer/navigation/boilerplate rather than a substantive document.

User-facing rendering:

- `blocked by site`
- `JS-rendered content`
- `redirect stub`
- `boilerplate-only content`

### Collection rejection error

A collection pipeline error representing safe rejection of a URL before artifact creation.

Fields:

- `url: String`
  - The normalized or attempted URL being collected.
- `reason: RejectReason`
  - Concise reason category.

User-facing rendering:

```text
<url> was not collected: <reason>
```

## Relationships

- `CollectError` owns/embeds the collection rejection error as one possible failure mode.
- `collect_url` may create a rejection from fetch-level conditions, e.g. `403`.
- `collect_html` may create a rejection from raw HTML or extracted markdown classification.
- `leaf::write` and `index::append_entry` must only be called after no rejection is present.

## Storage approach

No new persistent storage.

Rejected URLs are intentionally not recorded in the tree index for this feature. A failed URL can be retried later after the site changes or a future adapter/recovery feature is added.
