# Analysis: JSON output for CLI commands

## Risk assessment

### Broad CLI refactor risk

This feature touches the top-level command dispatcher, parsing, rendering, error handling, and every existing command. The highest risk is not JSON serialization itself; it is changing command control flow while preserving current human behavior.

Risk points:
- current command wrappers often print directly and return `Result<(), String>`;
- typed errors are lost early, which conflicts with structured JSON error mapping;
- `compile` prints inside the pipeline and has early successful exits;
- `raze` interleaves destructive actions with human output;
- `search` has different desired no-result exit behavior in human vs JSON mode.

Mitigation:
- introduce the runner/rendering boundary first;
- refactor commands one at a time behind tests;
- keep existing human render strings in dedicated render functions instead of rewriting UX opportunistically.

### Parser-level JSON errors may be awkward with clap

The spec requires JSON for usage/parsing errors when JSON mode was requested. This is more complex than ordinary command-level `--json` because clap normally owns parser errors and process exits.

Risk points:
- `--json` after subcommands must be accepted consistently;
- raw argv scanning can falsely detect positional `--json` unless it respects `--`;
- command inference from malformed argv will be heuristic;
- help/version behavior is explicitly out of scope but can interact with `--json`.

Mitigation:
- validate clap global arg behavior before code refactors;
- implement raw scan as a small, tested function;
- document that help/version remain clap-rendered human output even if `--json` appears, unless implementation naturally supports JSON errors there.

### Exact stdout cleanliness is brittle

The success criteria require stdout to contain only one JSON object in JSON mode. Any direct `println!` in command code violates this.

Risk points:
- `seed`, `collect`, `compile`, and `raze` currently print directly;
- existing `list/search/show` renderers print raw JSON if called directly;
- future commands may regress by printing in command logic;
- dependencies should not print to stdout, but this is outside strict control.

Mitigation:
- isolate all stdout writes in one dispatch/render layer;
- avoid calling old `render_json` functions from command modules directly;
- add regression tests that parse stdout and reject leading/trailing non-whitespace plus human markers.

### Error code stability can be overfit too early

The spec says the JSON contract is stable enough for scripts/agents. If implementation invents too many fine-grained codes ad hoc, later compatibility becomes painful.

Risk points:
- fetch/extract/YouTube errors may expose implementation-specific categories;
- opaque string errors may become accidental public API;
- mapping every internal compile validation warning to stable codes may freeze unstable internals.

Mitigation:
- define a small stable code set now;
- put implementation-specific context in `details` only when useful;
- use broader codes such as `fetch_error`, `extract_error`, `io_error` where subtyping is not externally actionable.

### Compile warning capture may expand scope

The plan mentions capturing actionable compile warnings. Current warnings are side effects in validation/execution. Capturing all of them structurally could require invasive changes.

Risk points:
- warnings are emitted from several internal compile functions;
- validation currently skips branches/leaves and only reports by `eprintln!`;
- making all warnings structured may alter control flow and tests.

Mitigation:
- minimum viable requirement: `leaves_skipped` is structured, no-op reasons are structured, and top-level warnings may initially cover only high-value skipped/degraded states already represented by returned data;
- leave low-level diagnostics on stderr unless they affect caller actionability.

### Existing tests may assume direct command functions

A new runner boundary may require test updates. Tests that call `cmd_compile` or other command wrappers may need to follow the new typed API.

Mitigation:
- preserve compatibility wrappers where cheap;
- add new typed functions alongside old human wrappers, then migrate main;
- only remove old functions after tests are adjusted.

## Gap analysis

### Help/version behavior is unresolved

Out of scope says CLI help/version output is not covered, but parser-level JSON errors are covered. This leaves ambiguous cases:
- `bo --json --help`
- `bo --json search --help`
- `bo --json --version` if version exists later

Recommendation:
- explicitly treat clap help/version as non-error human output, not JSON, for this feature.
- Add this to implementation notes or accept as a deliberate exception.

### Pretty vs compact JSON is unspecified

The spec requires parseable JSON and exactly one object, but not pretty vs compact. Existing renderers use pretty JSON.

Recommendation:
- use pretty JSON for human inspectability unless compact output is preferred for automation. Either is valid, but tests should parse JSON rather than snapshot whitespace.

### Top-level `warnings` duplication policy is vague

The spec says warnings include degraded-output notices. The data model says list rows retain per-row degradation and top-level warnings can summarize or duplicate.

Open implementation decision:
- Should every degraded row create a top-level warning?
- Should warnings be capped for large lists?
- Should warnings include a count summary instead?

Recommendation:
- for `list`, include one warning per degraded returned row because returned rows are already limit-filtered;
- avoid warnings for rows not included due to `--limit`.

### Exit code policy for JSON no-result/non-error cases is incomplete

The spec explicitly covers `search --json` no results and `compile --json` empty/single-leaf no-ops. It does not explicitly cover:
- `list --json` empty tree;
- `list --json --branch missing`;
- `seed --json` already seeded;
- `raze --json` output dir left in place because not empty/already absent.

Recommendation:
- treat all of the above as `ok: true` with exit code 0 if current human mode treats them as non-errors.

### Command payload exact field names should be treated as normative

The data model gives example payloads, but some fields are optional in prose (`path` required if available). Implementation could diverge.

Recommendation:
- treat `data-model.md` as normative for field names and requiredness where possible;
- if a field cannot be populated, use `null` rather than omitting it only if the model allows null;
- for `collect.path`, the output directory is known, so include it as required.

### JSON serialization of YAML frontmatter may be leaky

`show` currently stores frontmatter as `serde_yaml_ng::Mapping`. YAML can represent non-string keys and values not ideal for a JSON object.

Risk:
- JSON serialization may fail or produce awkward output for unusual YAML frontmatter.

Recommendation:
- validate current behavior before implementation;
- if needed, convert frontmatter to a JSON-compatible value/object with stringified keys or return a structured `invalid_frontmatter` error for unrepresentable cases.

### `collect` success tests are underspecified

`collect_url` performs network fetches for ordinary URLs and invokes summary generation. Tests should avoid external network and API dependencies.

Recommendation:
- test command JSON shape using lower-level fixture HTML paths only if exposed through a test helper, or focus initial CLI tests on duplicate/not-seeded/error paths;
- do not introduce network-dependent tests.

### `raze` transactional semantics are not specified

`raze` mutates multiple files. If a mid-command error occurs in JSON mode, the error envelope can report failure but not necessarily partial state.

Recommendation:
- keep existing behavior for mutation ordering;
- for failures, include the failed path/message when available;
- do not attempt rollback in this feature.

## Edge cases

### CLI parsing and JSON mode

- `bo --json` with no subcommand: should emit JSON `usage_error`, `command: "bo"`, nonzero.
- `bo --json nope`: should emit JSON `usage_error`, `command: "bo"` or `"nope"`; prefer `"bo"` because command is unknown.
- `bo search --json` with missing required terms: should emit JSON `usage_error`, `command: "search"`, nonzero.
- `bo search -- --json`: should not enter JSON mode solely because `--json` appears after `--`.
- `bo --json --help`: ambiguous due to help out of scope; should be decided before implementation.
- `bo search rust --json`: validate whether clap accepts this with global flag; if not, decide whether raw scan should still make parser errors JSON.

### Envelope integrity

- Success envelope must not include `error`.
- Error envelope must not include `data`.
- `warnings` must always be present, even empty.
- `details` must always be an object, not null/string.
- JSON output should end with a newline for terminal hygiene, but tests should parse independent of final newline.

### Command-specific behavior

- `seed --json` when config already exists should not overwrite anything and should return `status: "already_seeded"`.
- `collect --json` when not seeded should emit `not_seeded`, not a generic string error.
- `collect --json` duplicate URL should include `existing_file`.
- `compile --json` empty/single-leaf should not require `OPENAI_API_KEY`.
- `compile --json` with two leaves and missing API key should be structured failure.
- `list --json` with corrupted/missing leaf files should still return rows with degradation fields.
- `search --json` with no matches should exit 0, unlike human mode.
- `show --json` ambiguous errors should include candidates if available.
- `raze --json` with missing index should mirror current successful handling if index absence is currently tolerated.

### stdout/stderr separation

- Progress lines such as `fetching ...`, `summarizing...`, `writing branch...`, and warnings should not appear on stdout in JSON mode.
- Human mode should keep current progress/errors on stdout/stderr unless explicitly changed by the JSON refactor.
- Parser errors in JSON mode should avoid clap pre-printing its own human error to stderr if possible; stderr can contain diagnostics, but stdout must remain parseable.

## Dependencies

### Internal dependencies

- Existing command modules and result structs, especially `list`, `search`, and `show`, already use `serde::Serialize` and can be reused.
- `compile` internals are the main blocker because summary/no-op output is embedded in pipeline control flow.
- `main.rs` currently owns command wrappers for `seed`, `collect`, `list`, `search`, `show`, and `raze`; moving to typed results may require sizable edits in one file.
- `ConfigError::NotFound` must be preserved until JSON mapping to avoid generic config errors.

### External crates

- `clap` is the only external behavior dependency that needs validation.
- `serde_json` is already available for envelope serialization.
- `serde_yaml_ng` frontmatter serialization needs validation for JSON compatibility.
- No new runtime dependency is justified at this point.

### Environment dependencies

- `compile` normally needs `OPENAI_API_KEY` after no-op guards. Tests must avoid real LLM calls.
- `collect` can perform network I/O. Tests must avoid external network.
- Config path depends on HOME; existing tests already manage this with temp HOME and serial locks.

## Recommendation

Ready to implement after two preflight decisions/validations:

1. **Validate clap global `--json` behavior and parser-error interception first.** This determines the exact runner shape and prevents rework.
2. **Define the help/version exception explicitly.** Recommended rule: help/version remain clap/human output and are not part of JSON mode for this feature.

Implementation should proceed in vertical slices, not by refactoring every command first. Suggested order:

1. Add JSON envelope module and tests.
2. Add testable runner + parser-error JSON behavior.
3. Implement `list`, `search`, and `show` envelopes.
4. Refactor `seed`, `collect`, and `raze` typed results.
5. Refactor `compile` last.
6. Add cross-command contract tests and final cargo checks.

Do not start with `compile`; it has the largest control-flow changes and will obscure parser/envelope risks.
