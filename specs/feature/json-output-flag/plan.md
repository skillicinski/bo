# Implementation Plan: JSON output for CLI commands

## Architecture decisions and rationale

### Use one global JSON mode flag

Add one root/global `--json` flag instead of duplicating the flag independently in every command definition.

Rationale:
- Supports both `bo --json search rust` and `bo search --json rust`.
- Keeps CLI behavior consistent across commands.
- Avoids repeated argument plumbing as commands are added.
- Makes parser-level JSON errors possible because JSON mode can be detected before a subcommand is fully parsed.

### Keep command logic separate from rendering

Command functions should return typed outcomes. A final rendering layer decides whether to print human text or a JSON envelope.

Rationale:
- Prevents human decorations from leaking into JSON stdout.
- Makes command outcomes testable without parsing terminal text.
- Provides one place to enforce envelope shape and error formatting.

### Emit exactly one JSON object on stdout in JSON mode

JSON mode writes one envelope to stdout for both success and failure. Progress, diagnostics, and logs remain stderr-only.

Rationale:
- Agents and scripts can parse stdout directly.
- Nonzero exit codes still communicate failure to shells.
- Error details remain structured for corrective loops.

### Preserve human behavior by default

When `--json` is absent, existing human-readable output and exit-code behavior should remain unchanged unless a small internal refactor is required to keep JSON stdout clean.

Rationale:
- Avoids broad UX changes in a feature scoped to machine-readable output.
- Limits regression risk.

### Prefer typed errors over early stringification

Known domain/application errors should remain typed long enough to map them to stable JSON error codes. Human rendering can still use their existing `Display` messages.

Rationale:
- Error codes are more useful to agents than free-form strings.
- Preserves readable human errors.
- Allows command-specific details such as ambiguous candidates or duplicate filenames.

## Key components and responsibilities

### `main.rs` / CLI entrypoint

Responsibilities:
- Define global `--json` mode.
- Detect JSON mode before full clap parsing so parser errors can become JSON.
- Parse CLI args.
- Dispatch command execution.
- Render either human output or JSON output.
- Set exit codes consistently.

Expected behavior:
- Parser error + JSON mode detected: print JSON error envelope to stdout and exit nonzero.
- Parser error + no JSON mode: preserve clap's normal human error behavior.
- Command failure + JSON mode: print JSON error envelope to stdout and exit nonzero.
- Command failure + human mode: preserve current `error: ...` stderr behavior.

### JSON response module

A small reusable CLI-facing module should own:
- success envelope serialization
- error envelope serialization
- warning shape
- common error shape
- schema version constant

Responsibilities:
- Ensure every JSON response includes `schema_version`, `ok`, `command`, `warnings`, and either `data` or `error`.
- Serialize with `serde_json`.
- Keep payloads generic enough for command-specific typed data.

### Command result types

Each command should return a typed result payload suitable for both human rendering and JSON rendering:
- `seed`: seed status, output directory, tree name.
- `collect`: collected URL, filename, path if useful.
- `compile`: status, no-op reason when applicable, branches written, leaves updated, leaves skipped, warnings.
- `list`: existing list data plus top-level warning extraction if needed.
- `search`: existing search data plus query terms.
- `show`: existing show data.
- `raze`: removed counts and path/status details.

### Error mapping

Map typed errors to stable JSON error codes. Initial code set:
- `usage_error`
- `not_seeded`
- `already_seeded` if represented as a non-error status, otherwise not needed
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

The message should remain human-readable and actionable. Details should contain command-specific structured context when available.

### Compile pipeline

`compile` currently prints no-op and summary messages inside the pipeline. Refactor it to return `CompileSummary` or a new result enum instead of printing directly.

Responsibilities after refactor:
- Empty tree: return successful no-op result with reason `empty_tree`.
- Single usable leaf: return successful no-op result with reason `single_leaf`.
- Normal compile: return summary with branch results and skipped leaves.
- Human renderer prints existing compile messages.
- JSON renderer wraps the result in the common envelope.

Warning capture should be considered separately: warnings currently printed during validation/execution can remain stderr diagnostics, but warnings that affect interpretation should also be returned structurally when practical.

### Search command

Change JSON-mode no-results behavior to success.

Expected behavior:
- Human mode: preserve current no-results exit behavior unless intentionally changed later.
- JSON mode: `ok: true`, `hits: []`, exit 0.

## Integration points and external dependencies

### Existing dependencies

Use existing dependencies only where possible:
- `clap` for CLI parsing and global argument support.
- `serde` / `serde_json` for typed payload serialization.
- existing command/domain types where already serializable.

No new runtime dependency is expected for this feature.

### Clap integration

Validate:
- how to define a global flag with derive API;
- whether global flags are accepted after subcommands;
- how to intercept clap errors without immediate process exit;
- how to preserve normal clap behavior in human mode.

Likely approach:
- use `Cli::try_parse()` or equivalent instead of `Cli::parse()`;
- scan raw args for `--json` before `--` to decide JSON parser-error mode;
- if parsing succeeds, rely on parsed global flag for final JSON mode.

### Existing JSON renderers

`list`, `search`, and `show` already expose JSON rendering. Replace or wrap those outputs with the shared envelope so the external contract is consistent.

Implementation should avoid double-encoding JSON strings. Prefer serializable data types over pre-rendered strings.

## Implementation strategy

### Phase 1: Shared JSON infrastructure

- Add common envelope, warning, and error payload structs.
- Add helper functions for success/error rendering.
- Add command-name constants or a small enum if useful.
- Add unit tests for envelope shapes.

### Phase 2: CLI parsing and dispatch

- Add global `--json` support.
- Add pre-parse JSON-mode detection for parser errors.
- Replace direct `Cli::parse()` with error-aware parsing.
- Preserve clap's normal error output when JSON mode was not requested.

### Phase 3: Command result refactors

Refactor commands incrementally:
1. `list`, `search`, `show` — easiest because they already return structured data.
2. `seed`, `collect`, `raze` — move human print strings behind renderers.
3. `compile` — return typed summary/no-op outcomes instead of printing internally.

Each command should expose:
- a typed execution function;
- a human renderer;
- JSON rendering through the shared envelope.

### Phase 4: Error mapping

- Preserve typed errors until the dispatch layer can map them.
- Add structured details for high-value errors:
  - duplicate URL existing file;
  - rejected URL reason;
  - show ambiguity candidates;
  - usage parse context;
  - missing config / not seeded;
  - compile failure category.

### Phase 5: Tests and verification

Add tests for:
- every command accepts `--json` in command-local position and global position where supported;
- each command's successful JSON parses and uses the shared envelope;
- command failures emit JSON on stdout and exit nonzero;
- parser errors emit JSON when JSON mode is requested;
- stdout contains no human decorations in JSON mode;
- `search --json` no results exits 0 with empty `hits`;
- `compile --json` empty/single-leaf returns `ok: true` no-op status;
- human mode behavior remains unchanged for representative commands.

Use existing unit-test style where possible. If process-level CLI behavior is hard to test through unit functions, add a small integration-test harness around the built binary or a testable `run(args, stdout, stderr)` entrypoint.

## Risks and mitigations

### Parser-level JSON errors with command-local `--json`

Risk: malformed command input can prevent clap from recognizing command-local `--json`.

Mitigation: scan raw args for `--json` before full parsing. Treat `--json` before `--` as a JSON-mode request.

### Compile warnings are currently side effects

Risk: warnings printed via `eprintln!` are not captured in the JSON `warnings` array.

Mitigation: prioritize structurally important warnings in the returned compile result. Leave low-level diagnostics on stderr if not essential to result interpretation.

### Existing JSON renderers have non-envelope shapes

Risk: callers may already depend on old shapes for `list`, `search`, or `show`.

Mitigation: feature spec defines the new envelope as the stable contract. This is an intentional contract consolidation.

### Overly rich payloads

Risk: command payloads become deeply nested or include unnecessary internal details.

Mitigation: include only action-oriented fields needed by agents/scripts to retry, inspect, troubleshoot, or explain results.
