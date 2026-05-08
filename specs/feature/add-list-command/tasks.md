# Add List Command — Tasks

## Orchestration notes

- Complete T001–T003 first; they establish the branch, module boundary, and shared API.
- After T003, useful parallel waves:
  - **Domain agent**: T004–T013 (`src/list.rs` + unit tests). Keep this as one stream to avoid same-file conflicts.
  - **CLI/test agent**: T014–T021 (`src/main.rs` + CLI integration tests), using the API from T003. Tests may fail until domain work lands, but can be authored independently.
  - **Docs agent**: T023 (`README.md`) can run independently after command shape is fixed.
- Run T022 and T024 after implementation merges; they are final cross-cutting verification tasks.
- Avoid parallel edits to the same file unless explicitly coordinated.

## Setup and module boundary

- [x] T001 Verify current branch is `feature/add-list-command` and inspect `src/main.rs`, `src/lib.rs`, `src/index.rs`, `src/frontmatter.rs`, and existing CLI tests. Verify with `git branch --show-current` and targeted reads/rg.
- [x] T002 Add `src/list.rs` and export it with `pub mod list;` from `src/lib.rs`. Verify `cargo check` still compiles with an empty/skeleton module.
- [x] T003 Define the public list API skeleton in `src/list.rs`: `ListOptions`, `ListLeafRow`, `ListResult`, `ListError`, `list_leaves`, `render_human`, and `render_json`. Include derives needed for JSON serialization where appropriate. Verify with a minimal unit test or `cargo check`.

## Core list domain behavior

- [x] T004 Implement index loading in `list_leaves`, reading `{tree}/index.jsonl` via existing `index::read_index` and preserving index order through `index_position`. Verify with a unit test that default result order matches index order.
- [x] T005 Implement safe leaf path resolution under the tree root. Suspicious/path-traversal index entries must produce degraded rows and must not read outside the tree. Verify with a unit test using an index entry like `../outside.md`.
- [x] T006 Implement missing-file degradation. Indexed leaves whose files are absent should still produce rows using index fallback data. Verify with a unit test asserting `degraded == true` and reason includes `missing file`.
- [x] T007 Implement frontmatter parsing for readable leaves and display-title fallback order: non-empty leaf frontmatter `title`, then non-empty index title, then filename stem/filename. Verify with unit tests for each fallback path.
- [x] T008 Implement `collected_at` extraction and RFC3339 parsing. Missing or invalid `collected_at` should mark the row degraded; valid values should be retained for display/sorting. Verify with unit tests for valid, missing, and invalid values.
- [x] T009 Implement branch extraction from leaf frontmatter. Missing `branches` means `[]` and is not degraded; `branches: []` is normal; non-string branch values mark the row degraded while preserving valid string values. Verify with unit tests for all cases.
- [x] T010 Implement exact `--branch <branch>` filtering against derived branch arrays. Verify exact matching, non-matching partial strings, and missing branch no-result behavior with unit tests.
- [x] T011 Implement `--recent` sorting: valid dates first, newest to oldest; missing/invalid dates last; ties preserve index order. Verify with unit tests covering mixed valid/invalid/missing dates.
- [x] T012 Implement `--limit <n>` after filtering and sorting. Verify with unit tests that limit is applied after branch filtering and recent sorting.
- [x] T013 Add a read-only unit test around `list_leaves` that snapshots file contents/metadata before and after listing and verifies no tree files are created, modified, or deleted.

## Rendering

- [x] T014 Implement human-readable rendering for normal rows: display title/slug, collected date or placeholder, and branch array such as `[branch_a, branch_b]` or `[]`. Verify with renderer unit tests using fixed `ListResult` fixtures.
- [x] T015 Implement human-readable empty and no-result messages: empty tree reports no collected leaves; branch filter with no matches reports no matching leaves. Verify with renderer unit tests.
- [x] T016 Implement visible degraded-row rendering with a non-color-only marker/label such as `⚠ DEGRADED: <reasons>`. Verify with a renderer unit test that degraded output includes `DEGRADED`.
- [x] T017 Implement JSON rendering as an object-rooted payload containing `leaves`, with each row including `file`, `display_title`, `collected_at`, `branches`, `degraded`, and `degradation_reasons`. Verify by parsing renderer output with `serde_json` in a unit test.

## CLI wiring

- [x] T018 Add `List` to the clap `Commands` enum in `src/main.rs` with flags `--limit <n>`, `--recent`, `--branch <branch>`, and `--json`. Verify `cargo run -- list --help` shows the command and flags.
- [x] T019 Route `Commands::List` through `require_config()`, call `bo::list::list_leaves`, and print either `render_human` or `render_json` to stdout. Verify manually against a seeded temp tree or via an initial CLI integration test.
- [x] T020 Preserve existing CLI error conventions for list failures: errors print as `error: ...` to stderr and exit non-zero. Verify with the no-seed CLI integration test.

## CLI integration tests

- [x] T021 Add CLI integration test: `bo list` without seed fails with the existing seed hint. Verify with `cargo test --test integration_cli list_without_seed` or equivalent targeted test.
- [x] T022 Add CLI integration test: `bo list` on a seeded empty tree succeeds and reports that no leaves have been collected. Verify stdout contains the empty-tree message.
- [x] T023 Add CLI integration test: `bo list` on a synthetic tree prints leaves in index order, includes collected dates, and shows branch arrays including `[]`. Verify stdout ordering and substrings.
- [x] T024 Add CLI integration test: `bo list --limit 1` prints at most one leaf row. Verify by counting row/title occurrences rather than locking exact table formatting.
- [x] T025 Add CLI integration test: `bo list --branch <branch>` uses exact matching and `bo list --branch <missing>` reports no matching leaves without failing. Verify status success and stdout contents.
- [x] T026 Add CLI integration test: `bo list --json` emits parseable JSON with required per-row fields and degradation status. Verify with `serde_json`.
- [x] T027 Add CLI integration test for combined flags, e.g. `bo list --branch branch_a --recent --limit 5 --json`, verifying parseable JSON and consistent filtered/sorted/limited results.

## Documentation and final validation

- [x] T028 Update `README.md` command list to include `bo list` and its core flags. Verify README stays concise and matches implemented command names.
- [x] T029 Run `cargo fmt --check`. Fix formatting if needed.
- [x] T030 Run `cargo clippy --all-targets --all-features -- -D warnings`. Fix warnings.
- [x] T031 Run `cargo test`. Fix failures.
