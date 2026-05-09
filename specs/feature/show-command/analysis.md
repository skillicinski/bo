# Add Show Command — Analysis

## Risk assessment

- **Title lookup requires reading leaf files.** The requested lookup field is the leaf title, and the authoritative title lives in frontmatter. Missing, unreadable, invalid, or suspicious leaf files can therefore affect both matching and display. Implementation must avoid silently treating a broken indexed leaf as simply "not found" when the index title matches the requested title.
- **Ambiguity detection can be accidentally weakened.** The command must collect all exact case-insensitive title matches before selecting. Returning the first match would violate the duplicate-title requirement.
- **Raw frontmatter preservation is easy to lose.** Existing `frontmatter::parse` returns a parsed mapping and body, not the raw YAML text. Human output requires stored frontmatter as-is, so `show` needs a splitter that preserves the raw frontmatter block text.
- **JSON contract could drift from spec.** The spec says JSON should include the stored frontmatter; the plan/data model emphasize parsed frontmatter. To satisfy both agent usability and the wording, JSON should include parsed `frontmatter` and preferably `frontmatter_raw` unless implementation deliberately documents one as the stored representation.
- **Path traversal/symlink handling must match `bo list` safety.** Reimplementing safe path resolution risks subtle differences. The MVP can duplicate the list approach, but tests must cover traversal and non-reading outside the tree.
- **Preview truncation can split UTF-8 or markdown awkwardly.** Use character-boundary-safe truncation. Exact formatting is out of scope, but output must be deterministic and visibly marked when truncated.
- **Integration test file is already large.** Adding many show tests to `tests/integration_cli.rs` may make it noisy. Acceptable for this slice, but keep fixtures small and avoid locking layout.
- **Task numbering/orchestration notes are stale.** Orchestration says final validation is T027–T029 and CLI stream T016–T026, but tasks now run through T035. This is harmless but should be corrected during implementation or before handoff.

## Gap analysis

- **Broken candidate semantics need one explicit rule.** Recommended implementation rule:
  - If a candidate leaf can be read and parsed, match by frontmatter title with index-title fallback.
  - If a candidate leaf cannot be read/parsed but its index title exactly matches the requested title case-insensitively, return a display error for that file.
  - If a broken candidate's index title does not match, ignore it for this lookup.
  This avoids hiding a selected broken leaf as `not found` while keeping unrelated broken leaves from blocking the command.
- **JSON frontmatter representation should be tightened.** Add both:
  - `frontmatter`: parsed YAML object for machine use
  - `frontmatter_raw`: stored frontmatter text for exact inspection
  This best serves the progressive-disclosure use case.
- **Candidate details for ambiguous titles need minimal shape.** Use file path and title, optionally URL. No need to include body previews.
- **Preview size is intentionally unspecified.** Implementation may choose a constant such as a character limit. Tests should assert that long content is truncated and omitted tail is absent, not the exact byte count.
- **No README task for error behavior.** Fine; README can stay concise and only mention command shape/purpose.
- **No `research.md` needed.** No external technical unknowns or new dependencies.

## Edge cases

- Empty tree or missing `index.jsonl`: not found, suggest `bo list`.
- Request differs only by case: match.
- Request is a partial title: fail not found.
- Duplicate titles differing only by case: ambiguous.
- Title with leading/trailing whitespace in frontmatter: trim for matching/display identity.
- Empty frontmatter title with non-empty index title: index title can match.
- Empty frontmatter title and empty index title: cannot match by title.
- Missing file whose index title matches request: clear missing-file error.
- Invalid frontmatter whose index title matches request: clear invalid-frontmatter error.
- Suspicious path whose index title matches request: clear suspicious-path error and never read outside tree.
- Long Unicode body: truncate on character boundary.
- Short body: no truncation marker.
- Body exactly at preview boundary: no truncation marker.
- `--json --full`: full body, `truncated = false`, `full = true`.
- Titles beginning with `-`: clap users may need `bo show -- "-title"`; no special support required.

## Dependencies

- Existing `clap`, `serde`, `serde_json`, `serde_yaml_ng`, and filesystem APIs are sufficient.
- No network dependency.
- No LLM/API-key dependency.
- No branch-file dependency.
- No new crate needed.

## Recommendation

Ready to implement after two small task/spec hygiene fixes:

1. Update `tasks.md` orchestration notes to reference the actual task ranges ending at T035.
2. During implementation, include both parsed `frontmatter` and `frontmatter_raw` in JSON output to satisfy the spec and improve agent progressive disclosure.

Do not broaden lookup semantics. Keep MVP to exact case-insensitive title matching, leaf-only display, read-only behavior, default preview, `--full`, and `--json`.
