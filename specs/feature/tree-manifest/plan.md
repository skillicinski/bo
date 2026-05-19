# Plan: Tree Manifest (3a)

## Architecture Decisions

### 1. Manifest is the primary store; the secondary store is a safety net

Every mutating command writes the manifest first (the commit), then writes the secondary store. Reads come exclusively from the manifest. The secondary store exists in 3a for two reasons: (i) to allow reconstruction if the manifest is lost, and (ii) to keep the diff against v0.0.1 minimal so the read-path migration can be reviewed in isolation. It is removed in 3b.

### 2. Types + I/O live in one module

`src/domain/manifest.rs` holds the type definitions, atomic read/write functions, and resolution helpers. This matches the precedent set by `src/domain/index.rs` (types and I/O colocated in a single domain module) and avoids a premature engine/domain split for what is a single small surface.

### 3. Atomic write via tmp + rename

`manifest::write(path, &Manifest)` serializes to JSON, writes `{path}.tmp`, fsyncs, then renames over the final path. Internal helper, not exposed. Crash-safety guarantees beyond this single-file atomicity are 3b's responsibility.

### 4. Write order: manifest first, secondary second

Both must succeed; errors propagate. If the secondary write fails after the manifest commit, the manifest is canonical and consistent with what the user requested — only the safety net is stale, which is acceptable because reads no longer touch it.

### 5. Consumers migrate to `LeafRecord`

The eight call sites currently using `IndexEntry` (collect, status, list, search, show, query, plus tests) switch to `LeafRecord`. No conversion shim. `IndexEntry` and `read_index` remain in the codebase only because the secondary writers still emit them; nothing reads them after this feature lands.

### 6. Missing manifest is reconstructed from the secondary store

If `manifest::read(path)` finds the file absent, it attempts reconstruction from the secondary store before erroring. Reconstructed manifests are written back to disk so subsequent reads are direct. If the secondary store is also empty, reads error with `tree not initialized; run bo seed`. This affordance is deliberately temporary; the function carries a comment marking it for removal in 3b.

### 7. Compile pipeline materializes `Vec<BranchRecord>` as an intermediate

The validated LLM response is transformed once into `Vec<BranchRecord>`. Both the manifest writer and the branch `.md` frontmatter writer read from this in-memory structure. No double translation from raw response shape; no risk of the two on-disk representations being computed differently.

## Key Components

### `src/domain/manifest.rs` (new)

- Types: `Manifest`, `TreeMeta`, `LeafRecord`, `BranchRecord` (per `data-model.md`).
- `pub fn read(path: &Path) -> Result<Manifest, ManifestError>` — reads manifest, falls back to reconstruction if absent.
- `pub fn write(path: &Path, manifest: &Manifest) -> Result<(), ManifestError>` — atomic serialization.
- `impl Manifest`:
  - `leaf_by_slug(&self, slug: &str) -> Option<&LeafRecord>`
  - `branch_by_slug(&self, slug: &str) -> Option<&BranchRecord>`
  - `uncompiled_leaves(&self) -> Vec<&LeafRecord>` — leaves whose slug is in no branch's `leaves` list
  - `stale_branches(&self) -> Vec<&BranchRecord>` — `branch.stale == true` (always empty in 3a)
  - `leaves_for_branch(&self, branch_slug: &str) -> Vec<&LeafRecord>`
  - `branches_for_leaf(&self, leaf_slug: &str) -> Vec<&BranchRecord>` — inverse, computed at call time
- `fn reconstruct_from_secondary(tree: &Tree) -> Result<Manifest, ManifestError>` — private helper invoked by `read` on missing-manifest. Walks the secondary store and rebuilds.
- `pub enum ManifestError { Io(io::Error), Parse(serde_json::Error), TreeNotInitialized }` with `Display`/`From` impls matching the project's existing error patterns.

### `src/domain/tree.rs` (modified)

Add:
- `impl Tree { pub fn manifest_path(&self) -> PathBuf }`
- Free helper `pub fn manifest_path(tree_dir: &Path) -> PathBuf` mirroring `index_path` and `state_path`.

### `src/domain/mod.rs` (modified)

- `pub mod manifest;`

### `src/cli/seed.rs` (modified)

After existing seed work, write an empty manifest containing `TreeMeta` (name, created_at, last_compiled_at: null) and empty leaves/branches vectors.

### `src/cli/collect.rs` (modified)

- Read manifest for duplicate detection via `Manifest::leaves[].url`.
- After successful fetch + write of leaf `.md`:
  1. Append `LeafRecord` to manifest.leaves; `manifest::write`.
  2. Then append `IndexEntry` to `index.jsonl` (secondary).
- Both writes required; failure of either propagates.

### `src/cli/compile.rs` (modified)

- Read manifest at start (already reads tree state today).
- After validated LLM response:
  1. Materialize `Vec<BranchRecord>` from response.
  2. Build new `Manifest` value with updated `branches`, leaf-record updates as needed, and `tree.last_compiled_at` set.
  3. `manifest::write`.
  4. Write branch `.md` files (secondary).
  5. Update `state.json` (secondary).

### `src/cli/{status,list,search,show,query}.rs` (modified)

Replace `read_index(tree.index_path())` and `read_state(tree.state_path())` with `manifest::read(tree.manifest_path())`. Migrate downstream code to consume `&LeafRecord` / `&BranchRecord` directly. Remove any filesystem walks of `branches/` for metadata purposes (content reads remain).

### `src/cli/raze.rs` (modified)

Delete `manifest.json` as part of tree teardown.

### Tests

- `src/tests/domain_manifest_tests.rs` (new): round-trip read/write, atomic-write semantics under simulated rename failure, resolution helpers (each method against a fixture manifest), reconstruction from a tree with secondary files only.
- `src/tests/cli_collect_tests.rs` (modified): assert post-collect manifest matches secondary state (parity).
- `src/tests/cli_compile_tests.rs` (modified): assert post-compile manifest contains branches matching secondary state, `last_compiled_at` is set.
- `src/tests/cli_status_tests.rs` (modified): snapshot `bo status --json` against `src/tests/fixtures/status_snapshot.json` (checked-in expected output).
- `src/tests/cli_list_tests.rs` (modified or new if absent): snapshot `bo list --json` against `src/tests/fixtures/list_snapshot.json`.
- New integration test: build a tree, delete `index.jsonl` and `state.json`, run each read command, assert output unchanged.
- New integration test: build a tree, delete `manifest.json`, run a read command, assert reconstruction warning on stderr and successful output.

## Integration Points

- **No new dependencies.** `serde`, `serde_json`, `chrono` are already in `Cargo.toml`.
- **No CLI surface changes.** No new flags, no new error codes, no new prompts.
- **Configuration:** unchanged. Manifest path is derived from existing tree config.
- **Authentication:** unchanged.
- **Existing modules touched:** `src/cli/{seed,collect,compile,status,list,search,show,query,raze}.rs`, `src/domain/{tree,mod}.rs`. New module: `src/domain/manifest.rs`. New test files as listed above.

## Implementation Strategy

Sequenced to keep each step independently verifiable. Run `cargo test` after every step.

1. **Manifest module skeleton.** Create `src/domain/manifest.rs` with types, `read`, `write`, atomic-write helper, resolution helpers. Add to `src/domain/mod.rs`. Add `Tree::manifest_path` and the free helper. Unit tests for round-trip and resolution helpers. No callers yet.

2. **Reconstruction.** Implement `reconstruct_from_secondary` and wire it into `manifest::read`'s missing-file branch. Unit tests for reconstruction against a synthetic secondary-store fixture.

3. **`bo seed` writes manifest.** Smallest behavioural change. Add a manifest-existence assertion to seed's existing tests.

4. **`bo collect` dual-write + manifest-driven dedup.** Switch dedup source to manifest. Add manifest-write step. Existing collect tests should continue to pass; add a parity assertion comparing manifest.leaves to index.jsonl entries after a sequence of collects.

5. **`bo compile` dual-write.** Materialize `Vec<BranchRecord>` from validated response. Add manifest-write step. Existing compile tests continue passing; add a manifest-population assertion.

6. **Read-path migration, one command per commit.** In order: `status`, `list`, `search`, `show`, `query`. Each migrates its own consumer to `LeafRecord`/`BranchRecord` and removes its filesystem walks. Existing tests must continue to pass; snapshot tests for `status --json` and `list --json` are added at this stage.

7. **`bo raze` deletes manifest.** Add to existing raze tests.

8. **Integration tests for read robustness.** Two scenarios:
   - Delete secondary files; reads continue to produce identical output.
   - Delete manifest; reads succeed via reconstruction with a stderr warning.

9. **Cargo clippy + fmt + test green.** Final pass.

Each step is a clean commit. The PR can be reviewed step-by-step or as a whole. Total estimate: 600–900 lines of production code + tests.
