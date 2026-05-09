# Analysis: `bo search`

## Risk assessment

### Low risk
- **UTF-8 snippet boundaries** — plan already calls out `char_indices()`. Standard pattern, well-tested in Rust ecosystem.
- **Performance** — 50 leaves × ~20KB average = ~1MB of reads + substring matching. Well under 500ms on any modern disk.
- **Shell quoting** — clap handles this natively; no custom parsing needed.

### Medium risk
- **Scoring bias toward longer documents** — the plan specifies absolute occurrence count. A 15,000-word article mentioning "rust" 8 times will outscore a 300-word article mentioning "rust" 4 times, even though the short article is clearly more focused. The spec says "match density" but the plan implements raw count. These contradict each other.
- **Frontmatter field pollution in matching** — matching against the raw frontmatter text means every leaf matches searches for "title", "url", "collected_at", or "2025". Searching "example" matches any leaf collected from `example.com`. The spec says "body + frontmatter" without distinguishing between field names/syntax and field values.

### No real risk
- No new crates, no network, no mutation, no concurrency. Implementation is a straightforward pure function.

## Gap analysis

### 1. "Density" vs "count" — unresolved
Spec requirement #6 says "term-frequency/match-density". Plan says `content.matches(term).count()` summed. These are different things. Density normalizes by document length; count does not.

**Decision needed:** use raw count (simpler, biases toward long docs) or normalize by char/word count (fairer ranking, trivially more code)?

*Recommendation:* normalize by character count — `(total_occurrences * 1000) / content.len()` gives a per-mille density. One extra line, significantly better ranking for heterogeneous corpus sizes.

### 2. What is "frontmatter" for matching purposes?
Options:
- (a) The raw YAML text between `---` delimiters, including field names and syntax
- (b) Only the field *values* (title, url)
- (c) Only semantically meaningful values (title only; url is metadata not content)

Option (a) is simplest (just concat the whole file and search) but produces false positives on structural terms. Option (b) requires selective extraction. Option (c) is cleanest but requires a policy decision on each field.

**Decision needed.** The simplest correct answer: search the entire file content (option a). Accept that `bo search url` or `bo search "---"` returns every leaf. This matches grep/ripgrep behavior — you're searching file content, not a curated index. Document it as "searches the full leaf file including metadata."

### 3. Newlines in snippets
A ±80 char window will often span markdown line breaks. `"…end of paragraph.\n\nNext paragraph starts…"` looks noisy in compact terminal output.

**Decision needed:** preserve newlines as-is, or collapse `\n+` to single space in the rendered snippet?

*Recommendation:* collapse to single space for human output. Preserve raw in JSON output (consumers can handle it).

### 4. Exit code for valid-search-but-empty-page
If 7 results exist and user passes `--page 3` (pages of 5 → only 2 pages), is this exit 0 (search had results) or exit 1 (this page is empty)?

**Decision needed.** *Recommendation:* exit 1 — the user sees no results on screen. Simpler mental model. `total_results` in JSON still communicates that matches exist.

### 5. `score` field in JSON output
Exposing the raw score in JSON creates an implicit API contract. If scoring changes later (density normalization, BM25), consumers break.

**Decision needed:** expose `score` in JSON or omit it? *Recommendation:* include it — it's useful for debugging and agent consumption. Document as "internal ranking score, not stable across versions."

## Edge cases not covered by tasks

| Case | Expected behavior | Status |
|------|-------------------|--------|
| Leaf file missing on disk (index references it) | Skip silently, do not include in results | Not in tasks — add to orchestration |
| Leaf with invalid/missing frontmatter | Skip or search raw content? | Not specified |
| Empty body (frontmatter only, no markdown) | Snippet is empty string or "(no content)" | Not specified |
| All terms match but leaf is unreadable (permissions) | Skip silently | Not in tasks |
| Search term is empty string (`bo search ""`) | Match everything? Reject as usage error? | Not specified |
| Overlapping term matches (`bo search own ownership`) | "ownership" counts for both terms | Acceptable but undocumented |
| Very long search term (>1000 chars) | Works but slow on large corpus | Acceptable |
| `--page 0` | Error (page must be ≥1) | Covered in tasks |
| Single leaf matches all terms, rest don't | Pagination shows 1 result on page 1, empty page 2 | Implicit but not tested |

## Dependencies

None. Zero external factors:
- No new crates
- No network
- No config changes
- No schema migrations
- Reuses existing `domain::index` and `domain::frontmatter` which are stable

The only sequencing dependency is the merge of `refactor/replace-agent-with-workflow` → main (noted in last session). The worktree sidesteps this — we're branching from current main.

## Recommendation

**Ready to implement** with three decisions to lock in first:

1. **Scoring: count vs density** — recommend density (normalize by content length). One-line change, meaningfully better results for mixed-size corpora.
2. **Match corpus: whole file vs selective** — recommend whole file (option a). Simplest, matches grep behavior, no false negatives.
3. **Snippet newlines: preserve vs collapse** — recommend collapse to space for human output, preserve in JSON.

The edge cases (missing files, unreadable leaves, empty body) don't need spec changes — they're implementation details that follow from the `list.rs` "skip gracefully" pattern. Add them as sub-bullets under the orchestration task.

No blockers. Ship it.
