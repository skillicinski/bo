# Reject Low-Value Collections

## Problem statement

`bo collect <url>` can currently report success and write a markdown leaf even when the URL did not yield an acceptable document. Examples include JavaScript-only shells, anti-bot/block pages, redirect stubs, and boilerplate-only pages.

This makes the local tree less trustworthy: users must manually inspect collected leaves to discover that some are junk.

The feature should make collection fail safely when the fetched result is clearly not the intended document.

## User-facing requirements

- When a URL does not produce an acceptable document, `bo collect <url>` must reject it instead of writing a leaf.
- A rejected collection must:
  - exit non-zero
  - print a clear error to stderr
  - not create a markdown leaf
  - not append an index entry
- Rejection errors should use a consistent shape:
  - base message: `<url> was not collected:`
  - reason suffix, e.g. `blocked by site`, `JS-rendered content`, `redirect stub`, `boilerplate-only content`, or equivalent concise categories
- The first scope must reject at least these classes of bad collection:
  - JS-required/app shell pages, such as direct X/Twitter pages that do not expose tweet content
  - anti-bot, captcha, publisher block, or access challenge pages
  - deterministic redirect stubs that contain no document content
  - boilerplate/footer-only pages where the extracted text is not the intended document
- Pages with useful body content must not be rejected solely because their title is generic, wrong, or UI-derived.
- A low-quality title may contribute to suspicion only when the page content itself is also low-value.
- Existing successful article/document collection should continue to work.

## Success criteria

- Collecting a JS-required X/Twitter page does not write a leaf and reports a JS-rendered/content-unavailable style reason.
- Collecting a blocked Medium/Cloudflare-style page does not write a leaf and reports a blocked/access-challenge style reason.
- Collecting the Rust blog redirect stub does not write a leaf and reports a redirect-stub style reason.
- Collecting the observed OpenReview shell/footer-only result does not write a leaf and reports a boilerplate/low-value style reason.
- Rejected URLs leave the tree unchanged: no new markdown file and no new index row.
- Known-good article/document URLs in the default corpus still collect successfully.
- Rust book/mdBook pages with real chapter body content are not rejected only because the title is poor.

## Out of scope

- Following redirect stubs to their target document.
- Source-specific recovery adapters.
- X/Twitter collection through xcancel or any other mirror.
- YouTube transcript collection.
- OpenReview metadata/API extraction.
- Medium bypassing, authenticated fetching, or anti-bot circumvention.
- Title cleanup, title fallback, or title-quality warnings except where title quality is used as a secondary signal for low-value content.
- A `--force` or override mode.

## Open questions

None for first scope.
