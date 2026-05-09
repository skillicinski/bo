# Add Show Command — Tasks

## Orchestration notes

- Complete T001–T003 first; they establish the branch, module boundary, and shared API.
- After T003, useful parallel waves:
  - **Domain/rendering stream**: T004–T015 (`src/cli/show.rs` + unit tests). Keep this as one stream to avoid same-file conflicts.
  - **CLI/integration stream**: T020–T031 (`src/main.rs` + `tests/integration_cli.rs`), using the API from T003. Tests may fail until domain work lands.
- Run T032–T035 after implementation is complete; they are final cross-cutting validation tasks.
- Do not broaden lookup semantics beyond exact case-insensitive title matching.

## Setup and module boundary

- [x] T001 Verify current branch is `feature/show-command` and inspect `src/main.rs`, `src/cli/mod.rs`, `src/cli/list.rs`, `src/domain/index.rs`, `src/domain/frontmatter.rs`, and existing CLI tests. Verify with `git branch --show-current` and targeted reads/rg.
- [x] T002 Add `src/cli/show.rs` and export it with `pub mod show;` from `src/cli/mod.rs`. Verify `cargo check` still compiles with an empty/skeleton module.
- [x] T003 Define the public show API skeleton in `src/cli/show.rs`: `ShowOptions`, `ShowResult`, `ShowError`, `show_leaf`, `render_human`, and `render_json`. Include derives needed for JSON serialization where appropriate. Verify with `cargo check`.

## Core show domain behavior

- [x] T004 Implement index loading in `show_leaf`, reading `{tree}/index.jsonl` via existing `domain::index::read_index` and preserving candidate index order. Verify with a unit test that an empty index produces a not-found error suggesting `bo list`.
- [x] T005 Implement safe leaf path resolution under the tree root. Absolute paths, empty paths, parent traversal, and Windows prefix paths must be rejected and must not read outside the tree. Verify with a unit test using an entry like `../outside.md`.
- [x] T006 Implement leaf document loading and splitting into raw frontmatter text, parsed frontmatter mapping, and body. Verify with a unit test that raw frontmatter preserves the stored text used for human output.
- [x] T007 Implement title extraction and fallback: non-empty leaf frontmatter `title`, then non-empty index title. Verify with unit tests for frontmatter-title and index-title fallback matches.
- [x] T008 Implement exact case-insensitive title matching. Verify with unit tests that different casing matches and partial title strings do not match.
- [x] T009 Implement not-found behavior. Missing titles must produce a non-success `ShowError` whose message mentions the requested title and suggests `bo list`. Verify with a unit test.
- [x] T010 Implement duplicate-title ambiguity behavior. If multiple leaves match case-insensitively, return an ambiguity error containing candidate file/path/title details and do not choose a leaf. Verify with a unit test.
- [x] T011 Implement selected-leaf failure behavior for missing, unreadable, or invalid-frontmatter files. Verify with unit tests that errors include the file and clear reason.
- [x] T012 Implement default bounded preview behavior. Long bodies return a bounded preview with `truncated = true`; short bodies return the full short body with `truncated = false`. Verify with unit tests without locking exact terminal layout.
- [x] T013 Implement `full` behavior. `ShowOptions { full: true }` returns the complete body and `truncated = false`. Verify with a unit test.
- [x] T014 Add a read-only unit test around `show_leaf` that snapshots tree file contents/metadata before and after showing a leaf and verifies no tree files are created, modified, or deleted.
- [x] T015 Ensure JSON-serializable result fields include leaf identity, parsed frontmatter, returned body, `truncated`, and `full`. Verify by serializing a fixed result in a unit test.

## Rendering

- [x] T016 Implement human-readable rendering for preview mode: raw frontmatter as stored, followed by the body preview. Verify renderer output includes frontmatter and preview content.
- [x] T017 Implement visible truncation indication in human preview output when `truncated = true`. Verify output includes a non-color-only marker/message indicating omitted content.
- [x] T018 Implement human-readable rendering for full mode: raw frontmatter as stored, followed by the full body and no truncation marker. Verify with a renderer unit test.
- [x] T019 Implement JSON rendering as an object-rooted payload containing `leaf` with title, file, path, URL when available, parsed frontmatter, body, `truncated`, and `full`. Verify by parsing renderer output with `serde_json` in a unit test.

## CLI wiring

- [x] T020 Add `Show` to the clap `Commands` enum in `src/main.rs` with positional `title: String`, `--full`, and `--json`. Verify `cargo run -- show --help` shows the command and flags.
- [x] T021 Route `Commands::Show` through `require_config()`, call `bo::cli::show::show_leaf`, and print either `render_human` or `render_json` to stdout. Verify manually against a seeded temp tree or via an initial CLI integration test.
- [x] T022 Preserve existing CLI error conventions for show failures: errors print as `error: ...` to stderr and exit non-zero. Verify with no-seed and not-found CLI integration tests.

## CLI integration tests

- [x] T023 Add CLI integration helper for `bo show` in `tests/integration_cli.rs`, plus synthetic leaf fixtures with configurable title/body/frontmatter.
- [x] T024 Add CLI integration test: `bo show <title>` without seed fails with the existing seed hint. Verify stderr contains the seeded-tree hint.
- [x] T025 Add CLI integration test: `bo show "Some Title"` on a seeded synthetic tree succeeds and prints stored frontmatter plus a bounded body preview.
- [x] T026 Add CLI integration test: title matching is case-insensitive and exact. Verify lower/upper-case lookup succeeds, while partial title lookup exits non-zero.
- [x] T027 Add CLI integration test: `bo show --full "Some Title"` prints full body content that default preview omits.
- [x] T028 Add CLI integration test: `bo show --json "Some Title"` emits parseable JSON with required identity/frontmatter/body/truncation fields.
- [x] T029 Add CLI integration test: `bo show --json --full "Some Title"` emits parseable JSON with full body and `truncated = false`.
- [x] T030 Add CLI integration test: missing title exits non-zero, reports not found, and suggests `bo list`.
- [x] T031 Add CLI integration test: duplicated title exits non-zero and reports ambiguity with candidate details.

## Documentation and final validation

- [x] T032 Update `README.md` command list to include `bo show <title> [--full] [--json]` and its read-only purpose. Verify README stays concise and matches implemented command names.
- [x] T033 Run `cargo fmt --check`. Fix formatting if needed.
- [x] T034 Run `cargo clippy --all-targets --all-features -- -D warnings`. Fix warnings.
- [x] T035 Run `cargo test`. Fix failures.
