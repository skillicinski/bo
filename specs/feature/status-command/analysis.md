# Analysis: `bo status`

## Risk assessment

### 1. Migration corrupts existing trees (HIGH — mitigated by design)

The v0→v1 migration moves `index.jsonl`. If this fails midway (crash after move, before version write), the tree is in a broken state: index is in `.bo/` but version file says 0, so next run tries to move again (file not found at root).

**Mitigation:** The plan says "idempotent, checks preconditions." Implementation must check: if `.bo/index.jsonl` already exists but version file is absent, write the version file and skip the move. Test this exact scenario explicitly.

### 2. Test surface for index path change is large (MEDIUM)

30+ references to `"index.jsonl"` across 8 test files. Task 2b is mechanical but high-volume. A missed reference means a test passes vacuously (reads empty index from wrong path, gets 0 results, asserts nothing meaningful).

**Mitigation:** After 2b, grep for any remaining bare `"index.jsonl"` in test files that don't go through `.bo/`. A single `rg 'join\("index.jsonl"\)' src/` should return 0 hits post-migration.

### 3. `bo raze` doesn't know about `.bo/` (MEDIUM)

Raze currently: deletes leaves via index → deletes `index.jsonl` → tries `remove_dir(output_dir)`. After migration, `.bo/` is a subdirectory. `remove_dir` only works on empty directories — it will fail because `.bo/` contains files. Raze will leave the tree partially standing.

**Mitigation:** Raze must be updated to also delete `.bo/` contents (or use `remove_dir_all` on `.bo/` before attempting the parent). This is NOT listed in the current tasks. **Add explicit raze update to task 2b.**

### 4. Slug derivation from filenames (LOW)

The state file maps slug → timestamp. Compile currently works with filenames (e.g. `"my-leaf.md"`). Status needs to derive slugs from index entry filenames (strip `.md`). This is trivial but must be consistent: if a filename is `foo-bar.md`, the slug is `foo-bar`. The spec says "slug appears in the index" but the index stores `file` not `slug`.

**Mitigation:** Define clearly: slug = `entry.file.trim_end_matches(".md")`. Use this consistently in both compile (writing state) and status (reading state). Document in the state module.

## Gap analysis

### 1. `bo seed` creates no `.bo/` directory today

Seed currently only creates `output_dir` and writes config. It doesn't create `index.jsonl` at all — the first `bo collect` creates it lazily via `append_entry`. Post-migration, seed should create `.bo/` + version file. But should it also create an empty `index.jsonl`? The current code handles "index doesn't exist → return empty vec" gracefully. Probably fine to let collect still create it lazily inside `.bo/`.

**Decision needed:** Does seed create an empty `.bo/index.jsonl` or just `.bo/version`? Recommend: just `.bo/` dir + version file. Index remains lazily created.

### 2. Spec says "read-only" but migration mutates

The spec says "`bo status` never modifies the tree." But `ensure_current()` (which runs before status) DOES modify the tree (moves files, writes version). This isn't a contradiction — migration is infrastructure, not status logic — but it should be clear that status's read-only guarantee applies after migration completes.

**Decision needed:** None — just document that migration is a separate pre-step. The status pipeline itself is read-only.

### 3. What's a "leaf file on disk" for missing-entry detection?

The health check "leaf files on disk not in index" needs to distinguish leaves from other `.md` files. The tree root might contain a README.md or notes. Current heuristic: `{tree}/*.md` minus branches. But what about non-leaf markdown files users place in their tree?

**Decision needed:** Filter by frontmatter presence? Or just glob `*.md` and accept false positives? Recommend: any `.md` file in tree root with valid frontmatter containing `url:` and `collected_at:` fields is a leaf. Files without those fields are ignored. This avoids flagging READMEs.

### 4. Compile integration: which leaves go into state?

Compile processes `loaded_leaves` (successfully parsed) and skips some (bad frontmatter, missing files). The `CompileSummary` has `leaves_updated` count and `leaves_skipped` list. Should state include:
- Only successfully compiled leaves (loaded + written to branches)?
- All leaves that were *read* during compile (even if assigned to no branch)?

**Decision needed:** Recommend: all `loaded_leaves` filenames → state. They were ingested by the compile run even if the LLM didn't assign them to a branch. From status's perspective, "compiled" means "the compile pipeline has seen this leaf." Skipped leaves (bad frontmatter) should NOT go into state — they need attention.

### 5. State merge semantics

Task 3 says "merges with existing state (previously compiled leaves retained)." But what about leaves that were in state from a previous compile and then got deleted from the tree? State would reference a slug that no longer exists. Is that a problem?

**Decision needed:** No — status already handles this via orphan detection (compares index against filesystem). Stale state entries for deleted leaves are harmless; they just won't appear in the index, so they're never reported as "compiled." Could optionally prune state of entries not in index, but that's optimization, not correctness.

## Edge cases

1. **Empty tree (seeded, zero leaves):** Status should report 0/0 and hint to collect. No crashes on empty index, empty state, no branches dir.

2. **Tree with leaves but no branches dir:** `branches/` doesn't exist until first compile. Status must not error when scanning for branches — treat as 0 branches, no last_compiled_at.

3. **Branch with missing/unparseable frontmatter:** `read_compiled_at` already returns `None` for these. Status should still count the branch but exclude it from last_compiled_at derivation.

4. **Concurrent access:** User runs `bo collect` while `bo status` is reading. Not a real concern at CLI scale — but status should not crash on partial index writes (JSONL is append-only, partial last line is already handled by `read_index` which skips malformed lines).

5. **Very long uncompiled list:** 100+ uncompiled leaves in human output. The spec says "capped with overflow." Pick a display cap (e.g. 10) and show "and 90 more…"

6. **Tree dir is a symlink:** `std::fs::metadata` follows symlinks by default. Should work, but worth a sanity check in migration (don't break symlinked trees).

7. **`.bo/` already exists but is a file, not a directory:** Migration should fail with a clear error, not panic.

## Dependencies

No external blockers. All crates are already in `Cargo.toml`. No network calls. No API changes.

The only internal dependency worth noting: **task 2b (caller switchover) touches every CLI module.** If any other feature branches are in flight that touch `collect.rs`, `compile.rs`, etc., there will be merge conflicts. Check for active branches before starting 2b.

## Recommendation

**Ready to implement** with these pre-conditions:

1. **Add raze update to task 2b** — raze must handle `.bo/` subdirectory cleanup. This is a gap in the current task list.

2. **Decide on leaf detection heuristic** for missing-entry health check (recommend: require `url:` field in frontmatter to qualify as a leaf).

3. **Decide on seed behavior** — recommend seed creates `.bo/` + version only, index remains lazily created.

4. **Document slug derivation** — `filename.trim_end_matches(".md")` — and ensure compile + status use the same logic.

These are all small decisions, not blockers. Implementation can proceed with the recommendations above as defaults.
