# Tasks: `bo status`

## 1. Infrastructure modules

- [x] **1a. `engine::migrate` module** — version detection (`read_version`), migration runner (`ensure_current`), v0→v1 migration (create `.bo/`, move `index.jsonl`, write version file). Idempotent and partial-failure safe. Unit tests in `src/tests/engine_migrate_tests.rs`: fresh tree (no .bo/), already-migrated tree (no-op), partial migration recovery, version-too-new error.

- [x] **1b. `engine::state` module** — `TreeState` struct with `compiled_leaves: HashMap<String, String>`, `read_state(path)` and `write_state(path, state)` functions. Absent file returns empty state. Malformed file warns to stderr and returns empty. Unit tests in `src/tests/engine_state_tests.rs`: read/write round-trip, absent file, malformed file graceful degradation.

## 2. Tree layout migration

- [x] **2a. Path helpers + seed update** — Add `Tree::infra_dir()`, `Tree::index_path()`, `Tree::state_path()` methods. Update `bo seed` to create `{tree}/.bo/` directory, write version file with `1`, place empty `index.jsonl` inside `.bo/`. Existing tests remain green (no read-path changes yet). Verify: `bo seed` produces correct new layout.

- [x] **2b. Migration integration + caller switchover** — Wire `ensure_current()` call early in every tree-touching command (collect, compile, list, search, query, show, raze). Replace all `tree_dir.join("index.jsonl")` references with `Tree::index_path()` (or equivalent). Update `bo raze` to delete `.bo/` directory contents (state.json, index.jsonl, version) before attempting `remove_dir` on the tree root. Update existing test fixtures and assertions that assume index at tree root. Verify: `cargo test` green, manual run against a v0 tree triggers migration and commands succeed, `bo raze` cleanly removes a v1 tree.

## 3. Compile integration

- [x] **3. Write compile state** — At the end of a successful `bo compile` run, derive leaf slugs from filenames processed, call `write_state()` to persist them with the run timestamp. Merges with existing state (previously compiled leaves retained). Test in `src/tests/cli_compile_tests.rs`: after compile, state.json contains expected slugs and timestamps.

## 4. Status command

- [x] **4a. `cli::status` module** — `StatusResult` struct, pipeline function: read index → read state → scan filesystem → compute health (orphans, missing) → derive uncompiled set → calculate size/tokens → generate hints → return `StatusResult`. Unit tests in `src/tests/cli_status_tests.rs`: known fixture trees exercise each data point and health scenario.

- [x] **4b. Output formatting + CLI wiring** — Human formatter (compact summary block with overflow cap on uncompiled list). JSON serialisation matching spec'd shape. Add `Status` variant to `Commands` enum in `main.rs`, dispatch to `cli::status`. Verify: `bo status` and `bo status --json` produce correct output against a real tree.

- [x] **4c. Integration test** — End-to-end: seed → collect 3 URLs → status (shows 3 uncompiled) → compile → status (shows 0 uncompiled, last_compiled_at set). Delete a leaf file → status reports orphan with remediation. Add a .md file outside index → status reports missing entry. Lives in `src/tests/cli_status_tests.rs` or a dedicated integration test file.

## Dependency graph

```
1a (migrate) ──┐
               ├──→ 2a (helpers + seed) ──→ 2b (switchover) ──→ 3 (compile state) ──→ 4a ──→ 4b ──→ 4c
1b (state)  ───┘
```

Tasks 1a and 1b are independent. Everything else is sequential.
