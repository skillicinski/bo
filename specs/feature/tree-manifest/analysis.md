# Analysis: Tree Manifest (3a)

Critical pre-implementation review of `spec.md`, `plan.md`, `data-model.md`, and `tasks.md` against the current codebase.

**Status:** All blocking issues resolved. Spec is implementable. T1.1 unblocked.

**Resolution summary** (2026-05-19):
- B1 dropped per-leaf `compiled_at`; uncompiled now derived from `tree.last_compiled_at` + `leaf.collected_at`. v0.0.1 unparseable-frontmatter quirk intentionally retired.
- B2 confirmed: `bo show` does not work on branches today; nothing to preserve. T5.2 leaf-only.
- B3 T3.1 dropped; replaced with one-shot manual byte-equivalence diff in T8.1.
- B4 `BranchRecord.compiled_at` renamed to `created_at`; `updated_at` added; branch frontmatter renamed in lockstep.
- S1 absorbed into B4.
- E1–E9 addressed individually in updated `tasks.md`.

The original analysis below is preserved as the audit record.

---

## Blocking issues (must address before coding)

### B1. "Uncompiled" semantic drift breaks byte-equivalence

**Problem.** Today `state.json` stores `compiled_leaves: HashMap<slug, timestamp>` — a per-leaf record set during compile for **every valid leaf seen**, regardless of whether that leaf made it into a branch (`src/cli/compile.rs` ≈ line 261). Status currently computes `uncompiled` as "leaf slug absent from `state.compiled_leaves`."

The data model and tasks redefine `uncompiled` as "leaf slug absent from any `BranchRecord.leaves`." These are **not equivalent**: a leaf that the LLM saw but did not assign to any branch (because it's a singleton concept and the >=2-leaves-per-branch rule excluded it) is `compiled` under v0.0.1 but `uncompiled` under the new definition.

**Impact.**
- Violates user requirement 7 (no behavior change).
- Breaks T3.1 snapshot byte-equivalence whenever the fixture has a singleton-concept leaf.
- Status would show users a different "uncompiled" count after a compile that excluded some leaves.

**Fix.** Add `LeafRecord.compiled_at: Option<String>` to the schema. Compile sets it for every valid leaf it processed (mirroring `state.compiled_leaves` exactly). `uncompiled_leaves()` returns leaves where `compiled_at.is_none()`. Membership in branches becomes orthogonal to "uncompiled."

Tasks affected:
- `data-model.md` — add the field.
- T4.3 — set `compiled_at` for every valid leaf during compile (regardless of branch assignment).
- T5.5 — implement `uncompiled_leaves()` over `compiled_at.is_none()`, not over branch membership.
- T2.1 reconstruction — populate `compiled_at` from `state.compiled_leaves` map (slug → timestamp lookup).

### B2. `bo show` on branches is **new behavior**

**Problem.** T5.2 says "show on a branch slug works (consults `Manifest::branch_by_slug`)." But `src/cli/show.rs::show_leaf` is leaf-only today; there is no branch path. Adding branch support introduces user-visible behavior, violating requirement 7.

**Fix.** Remove the branch-show requirement from T5.2. `bo show` migrates to manifest reads for leaves only in 3a. Branch-show is a separate feature, candidate for v0.0.2 item 5 (CLI rework) or later.

### B3. Snapshot determinism is non-trivial; T3.1 underspecified

**Problem.** `bo status --json` and `bo list --json` outputs embed timestamps (`collected_at`, `compiled_at`, derived `last_compiled_at`) and byte counts. These are non-deterministic across:
- real `seed` (uses `Utc::now()`)
- real `collect` (uses `Utc::now()`)
- real `compile` (uses `Utc::now()`)
- file content (frontmatter timestamps embedded in `.md` change byte size)
- `fs::read_dir` order (OS-dependent for some health-report fields)

T3.1 says "build a fixed fixture tree (deterministic content; no real network) inside a test helper" but doesn't acknowledge that going through the public commands cannot produce a deterministic byte snapshot.

**Fix.** T3.1's fixture is **hand-constructed**: write `manifest.json` (with hardcoded timestamps), the secondary files (`index.jsonl`, `state.json`, leaf `.md` files with frontmatter using fixed timestamps), and `branches/*.md` directly. The fixture also must:
- write **both** manifest and secondary store, so the same fixture survives the read migration (status/list code reads either side and produces identical output).
- pin `fs::read_dir`-derived order. Status's `health.missing_from_index` uses directory iteration. Either deterministic tree (no surprise files) or sort the output before snapshotting.

Update T3.1 to call this out explicitly. The fixture is committed as a directory layout in `src/tests/fixtures/manifest_tree/` plus the two `*_snapshot.json` files.

### B4. Compile must preserve `BranchRecord.compiled_at` across recompiles

**Problem.** `src/cli/compile.rs::execute_plan` preserves the existing branch's `compiled_at` field (`branch::read_compiled_at(&path).unwrap_or_else(|| run_timestamp)`). The branch frontmatter's `compiled_at` is the **first-compile timestamp**, never updated; only `updated_at` moves.

T4.3 says "Materialize a `Vec<BranchRecord>` from the validated LLM response." Without preservation logic, every recompile resets `compiled_at` to `run_timestamp` in the manifest, diverging from frontmatter (where it's preserved).

**Impact.** Manifest and frontmatter disagree on `compiled_at` after the second compile of a tree. Status's `last_compiled_at` (max of `compiled_at` across branches) would jump forward in the manifest but not in frontmatter. Snapshot byte-equivalence breaks on multi-compile fixtures.

**Fix.** T4.3 explicitly: for each branch in the validated plan, look up the existing `BranchRecord` in the previous manifest (or fall back to `branch::read_compiled_at` for trees pre-3a). Use the existing `compiled_at` if present, else `run_timestamp`. Mirror frontmatter exactly.

---

## Should-fix (recommend before coding)

### S1. `BranchRecord` is missing `updated_at`

**Problem.** Branch frontmatter has both `compiled_at` (first compile, preserved) and `updated_at` (every compile). Manifest only carries `compiled_at`. The dual-write produces two on-disk shapes that disagree on which timestamp fields exist.

**Impact.** Information loss. Item 4 (incremental compile) will need "when did this branch last change" for staleness — without `updated_at`, item 4 has to re-derive it.

**Fix.** Add `BranchRecord.updated_at: String`. T4.3 sets it to `run_timestamp` on every compile. Cheap fidelity improvement; future-proofs the schema for item 4.

### S2. `tree.last_compiled_at` reconstruction source is misnamed

**Problem.** `data-model.md` reconstruction step 3 says "Reading the secondary compile-state file if present → `tree.last_compiled_at`." But `state.json` carries `compiled_leaves: HashMap<slug, timestamp>`, not a tree-level field. The current user-facing "last compile" in status is `max(branch.compiled_at)` derived from a branches-dir scan.

**Fix.** `data-model.md` reconstruction: `tree.last_compiled_at = max(BranchRecord.compiled_at)` across reconstructed branches, or `None` if no branches exist. Same as what status does today; preserves byte-equivalence. Update step 3.

---

## Edge cases / underspecified behavior

### E1. Manifest write happens before secondary write — failure window

T4.2 / T4.3 spec writes "manifest first, secondary second; both must succeed." If the secondary write fails after the manifest commit, the command exits with an error but manifest has the new entry. Side effects:

- For collect: leaf `.md` is on disk, manifest knows about it, `index.jsonl` doesn't. Post-T5.6 (manifest dedup), this is consistent. **Between T4.2 landing and T5.6 landing**, dedup still uses `index.jsonl` → user can re-collect the URL → second leaf with same URL inserted into manifest (manifest doesn't enforce URL uniqueness either). Risk window during the migration's middle.
- For compile: branch metadata in manifest, branch `.md` file may or may not exist. Status's branch count uses manifest (post-T5.5) but content reads (e.g., query) hit the file directly.

**Recommendation.** Document this as a known transitional risk. Don't try to fix it — 3b's stage-then-commit-then-rename does. If it bothers you, sequence T5.6 immediately after T4.2 (move it from Phase 5 to Phase 4) so the dedup path is consistent before any other read migrates. Cheap rearrangement.

### E2. Concurrent `bo` invocations during 3a

No advisory lock until 3b. Two collects running concurrently can race the manifest read-modify-write. The latter's write wins; the earlier's leaf entry is lost. POSIX rename atomicity does not help — the loss is at the application level.

**Recommendation.** Document as known limitation. Single-writer assumption is in place per ADR-004 but enforcement only lands in 3b. Acceptable since you're the only user and not running parallel bo processes.

### E3. Leaf frontmatter `branches:` field can drift from manifest

Today, leaf frontmatter has a `branches: [...]` field set by compile (`patch_fields` in `execute_plan`). After 3a, the canonical answer to "which branches contain leaf X" is `Manifest::branches_for_leaf`. The frontmatter is a redundant copy.

If a partial-failure leaves frontmatter and manifest disagreeing, or if a future operation updates branches but not leaf frontmatter, the two diverge. Not fixed in 3a; not breaking in 3a (list reads from manifest after T5.1, computes `branches_for_leaf`, ignores frontmatter).

**Recommendation.** Document as expected: leaf frontmatter `branches:` becomes a regenerable projection, like branch frontmatter. Compile keeps writing it for now (secondary). 3b removes it.

### E4. `LeafRecord` field synthesis — capture point in collect

`write_new_document_with_summary_result` synthesizes `slug`, `collected_at`, `summary` as locals, returns a `Document { url, filename }`. T4.2 needs all of these for `LeafRecord` construction. Current return type loses information.

**Recommendation.** Either:
- (i) Extend `Document` to carry `slug`, `collected_at`, `summary`, `title`. Caller assembles `LeafRecord`.
- (ii) Have `write_new_document_with_summary_result` take a `&mut Manifest` parameter and append the record itself.

(i) keeps separation of concerns. (ii) is fewer lines. Pick one in T4.2's task description; current text is silent.

### E5. `TreeConfig.name` and `created_at` are `Option<String>` — what if `None`?

Reconstruction reads `tree.name` and `tree.created_at` from `~/.bo/config.json` (`TreeConfig`). Both fields are `Option<String>` for compatibility with old configs. `Manifest::TreeMeta::{name, created_at}` are non-optional.

For fresh-tree-only v0.0.2 this never triggers — `bo seed` always writes both. But reconstruction must materialize a `String`. Sensible fallbacks:
- `name` → directory basename (mirrors `Tree::from_config` fallback).
- `created_at` → `Utc::now()` rendered as ISO-8601, with a stderr warning ("tree config missing created_at; reconstructed at <now>").

**Recommendation.** T2.1 adds explicit fallback handling. Or: hard error "config missing required fields; reseed." Either is fine; pick one.

### E6. `manifest::write` does not enforce slug uniqueness

`data-model.md` invariant 1 says no duplicate leaf or branch slugs. `manifest::write` doesn't check. A buggy mutator could insert a duplicate; the file would deserialize fine; later `leaf_by_slug` would silently return the first match.

**Recommendation.** Add `debug_assert!` in `manifest::write` that panics in debug builds when uniqueness is violated. Documents the invariant in code; zero release-build cost. Two lines.

### E7. T1.2's "atomic-write" test claim is too strong

Task text says "test that `manifest::write` is atomic ... simulate by writing twice and checking only the second content is observed." That's testing **replacement**, not atomicity. True atomicity (crash-safe rename) cannot be unit-tested without OS injection.

**Recommendation.** Reword T1.2's test description: "verify the implementation uses `rename(tmp, final)` and does not leak `*.tmp` files on the success path. Crash-safety beyond filesystem rename atomicity is 3b's domain."

### E8. Manifest corruption (parse error) — no fallback to reconstruction

T1.2 returns `Parse` error when manifest exists but is invalid JSON. T2.1 reconstructs only when the file is **absent**. What if the manifest is corrupted (truncated, hand-edited)?

**Recommendation.** Don't auto-reconstruct on parse error; that's a footgun (silently overwrites whatever the user broke). Surface the parse error verbatim. User can `rm manifest.json` if they want reconstruction. Document this in the function comment. No task change needed; just be explicit.

### E9. Reconstructed manifest needs to match canonical write format

T2.1 reconstructs and persists. If the persisted manifest doesn't match what a fresh seed+collect+compile would produce (field ordering, whitespace), subsequent reads of the rewritten file might surface differences. `serde_json::to_string_pretty` is deterministic given a fixed struct, so this is fine — but T2.1 should explicitly assert that reconstruction → write → read round-trips bit-perfect.

---

## Dependencies and external factors

### D1. `compile` does not migrate reads in 3a

User requirement 4 lists status/list/show/query/collect-dedup. **Compile is absent.** This is correct per the spec, but it means:
- After 3a, compile still reads `index.jsonl` directly. Cannot benefit from manifest reconstruction.
- Item 4 (incremental compile) is the natural place to migrate compile's reads — it has to consult the manifest anyway to know what's uncompiled.

Document explicitly in `analysis.md` and reference from item 4's spec when that work begins.

### D2. Crate-root integration tests in `tests/`

`tests/integration_*.rs` exercise the public commands end-to-end. Several touch `index.jsonl` and `state.json` directly. The migration may break them in subtle ways (e.g., a test that asserts on `state.json` content after compile). Each task should run `cargo test` (already specified) — no extra work, just be aware that `tests/integration_compile.rs`, `tests/integration_status.rs`, etc. need to stay green and may need fixture updates.

### D3. No new crate dependencies

Confirmed — `serde`, `serde_json`, `chrono`, `serde_yaml_ng` all in tree. No `Cargo.toml` changes.

### D4. CHANGELOG.md current state

Need to verify whether `## [Unreleased]` section already exists. If not, T8.1 creates it under the v0.0.1 entry.

---

## Risk assessment summary

| Risk | Severity | Mitigation |
|---|---|---|
| B1 — uncompiled semantic drift | High (breaks behavior + snapshot test) | Add `LeafRecord.compiled_at`, update T4.3 + T5.5 |
| B2 — show-on-branch new behavior | High (violates user req 7) | Drop from T5.2 |
| B3 — snapshot non-determinism | High (T3.1 unimplementable as written) | Hand-built fixture, dual-write the fixture |
| B4 — branch `compiled_at` not preserved | High (manifest/frontmatter divergence) | T4.3 reads existing record before overwriting |
| S1 — missing `updated_at` on branches | Medium (information loss) | Add field; cheap |
| S2 — reconstruction source naming | Low (factual correctness in spec) | Update `data-model.md` |
| E1 — write-failure window during T4.2 → T5.6 | Low (transitional) | Move T5.6 earlier or accept |
| E2 — concurrent invocations | Low (you're the only user) | Document; 3b fixes |
| E5 — config `Option<String>` fallback | Low (fresh-tree-only) | Pick a strategy in T2.1 |
| E6 — slug uniqueness not enforced | Low (defensive) | `debug_assert!` in `write` |

---

## Recommendation

**Do not start T1.1 yet.** Apply the following deltas first (~30 minutes of editing):

1. **`data-model.md`:**
   - `LeafRecord`: add `compiled_at: Option<String>` field with `#[serde(default)]`.
   - `BranchRecord`: add `updated_at: String` field.
   - "Reconstruction" section step 3: change to "Reading the secondary compile-state file if present → populate `LeafRecord.compiled_at` per slug; compute `tree.last_compiled_at = max(BranchRecord.compiled_at)`."
   - Add invariant 6: "Slug uniqueness checked in `manifest::write` via `debug_assert!`."

2. **`tasks.md`:**
   - **T1.2:** Reword atomic-write test to "verify `rename` is the final operation; no `*.tmp` leak on success." Add `debug_assert!` for slug uniqueness in `manifest::write`.
   - **T1.3:** Update `uncompiled_leaves` description to "returns leaves where `compiled_at.is_none()`."
   - **T2.1:** Specify fallbacks for `TreeConfig.name`/`created_at` if `None`. Add note that parse errors do **not** trigger reconstruction (only missing-file does).
   - **T3.1:** Rewrite. Fixture is hand-constructed in `src/tests/fixtures/manifest_tree/`: write `manifest.json` AND secondary files with hardcoded timestamps. Snapshot tests load this fixture and assert against `status_snapshot.json` / `list_snapshot.json`. Same fixture survives the read migration.
   - **T4.3:** Add explicit step: "preserve `BranchRecord.compiled_at` from existing manifest entry (or via `branch::read_compiled_at` for first compile after 3a lands). Set `BranchRecord.updated_at = run_timestamp`. Set `LeafRecord.compiled_at = run_timestamp` for every valid leaf processed (mirrors `state.compiled_leaves`)."
   - **T5.2:** Drop "show on a branch slug works." Show stays leaf-only.
   - **T5.5:** Update to "compute `uncompiled` via `Manifest::uncompiled_leaves()` (which checks `LeafRecord.compiled_at.is_none()`); `last_compiled_at` from `manifest.tree.last_compiled_at` (set by compile, mirrored to manifest)."

3. **Optionally:** Move T5.6 earlier in Phase 4 (right after T4.2) to close the dual-source dedup window. Not strictly required.

Once those deltas land, the spec is implementable as written and T1.1 can begin.

The architectural decisions in ADR-004 and `plan.md` are sound. The gaps are mechanical: schema fields that didn't get carried through from current behavior, and one task description (T3.1) that promised more than the test mechanism can deliver.
