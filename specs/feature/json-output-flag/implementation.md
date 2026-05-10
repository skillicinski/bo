# Implementation Summary: JSON output for CLI commands

## Branch

`feature/json-output-flag`

## Scope implemented

Implemented a global `--json` mode for all current output-producing commands:

- `seed`
- `collect`
- `compile`
- `list`
- `search`
- `show`
- `raze`

JSON mode now emits a single structured JSON envelope on stdout for successes and failures. Human output remains the default when `--json` is absent.

## Main changes

### Shared JSON envelope

Added `src/cli/json.rs` with reusable JSON response infrastructure:

- `schema_version`
- `ok`
- `command`
- `data` for success
- `error` for failure
- `warnings`
- `JsonError`
- `JsonWarning`

Both success and error helpers serialize exactly one JSON object.

### CLI parsing and runner

Refactored `src/main.rs` around a testable runner boundary:

```rust
run_from(args, stdout, stderr) -> exit_code
```

This allows parser errors, stdout/stderr separation, and exit codes to be tested without subprocess-only tests.

Added one global clap flag:

```rust
#[arg(long, global = true)]
json: bool
```

Validated support for both:

```bash
bo --json list
bo list --json
```

Raw argv scanning detects `--json` before `--`, so parser-level failures can also emit JSON.

Help/version output remains clap/human output, matching the spec's out-of-scope boundary.

### Command results and rendering

Moved command behavior toward typed execution results plus render functions:

- `seed` returns status `created` / `already_seeded`
- `collect` returns URL, relative file, and full path
- `list` wraps existing list result in the envelope
- `search` wraps existing search result and adds query terms
- `show` wraps selected leaf under `data.leaf`
- `raze` returns deletion summary and warning list
- `compile` returns typed `CompileResult`

Human renderers preserve existing behavior where `--json` is absent.

### Compile refactor

Refactored `src/cli/compile.rs` so compile no longer prints from the core pipeline.

Added:

- `CompileResult`
- status values: `compiled`, `noop`
- no-op reasons: `empty_tree`, `single_leaf`
- serializable `BranchResult`
- `run_compile(cfg)` returning `Result<CompileResult, CompileError>`
- `cmd_compile(cfg)` compatibility wrapper for existing callers/tests

Empty and single-leaf compiles now return successful no-op JSON and do not require `OPENAI_API_KEY`.

### Error mapping

Added CLI-facing structured error mapping for known failures:

- `usage_error`
- `not_seeded`
- `duplicate_url`
- `rejected`
- `fetch_error`
- `extract_error`
- `youtube_error`
- `not_found`
- `ambiguous`
- `io_error`
- `json_error`
- `llm_error`
- `validation_error`
- `context_overflow`
- `truncated`
- `content_filter`
- `unknown_error`

High-value details are included where available, e.g. duplicate collect `existing_file`, show ambiguity candidates, parser error kind/exit code.

### Warnings

Implemented structured top-level warnings for:

- degraded list rows
- skipped compile leaves
- suspicious raze ledger entries

Per-row list degradation fields are preserved.

## Behavior changes

Intentional behavior changes:

- Existing `list`, `search`, and `show` JSON outputs now use the shared envelope instead of raw payloads.
- `bo search --json <query>` with no results exits `0` and returns `hits: []`.
- Parser-level errors emit JSON on stdout when `--json` appears before `--`.

Human mode behavior remains compatible.

## Tests added/updated

Added unit coverage for:

- JSON envelope serialization
- parser-level JSON errors
- raw `--json` detection stopping at `--`
- global and command-local `--json`
- seed created/already-seeded JSON
- all output commands accepting `--json`
- search JSON no-results success
- search JSON usage error
- show JSON not-found and ambiguous errors
- collect duplicate URL JSON error
- compile empty/single-leaf no-op JSON
- compile missing API key JSON error
- raze JSON summary and suspicious ledger warning

Updated integration tests for the new enveloped JSON shape.

## Verification

Passed:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Manual smoke tests covered:

- JSON command failure
- parser-level JSON error
- human `seed`
- JSON `search` no-results success

## Files changed

- `src/cli/json.rs` — new JSON envelope module
- `src/cli/mod.rs` — exports JSON module
- `src/main.rs` — global JSON mode, runner, command result/render flow, error mapping, tests
- `src/cli/compile.rs` — typed compile result/no-op refactor
- `tests/integration_cli.rs` — updated list/show JSON expectations
- `tests/integration_search.rs` — updated search JSON expectations and no-result exit code
- `specs/feature/json-output-flag/tasks.md` — all tasks checked off

## Notes

The implementation is intentionally CLI-layer-heavy. No persistent storage format changed. No new runtime dependency was added.
