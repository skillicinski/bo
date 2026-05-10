# Research: JSON output for CLI commands

## Technical unknowns to validate

### Clap global flag behavior

Validate:
- `#[arg(long, global = true)]` or equivalent works with derive-based subcommands.
- `bo --json search rust` and `bo search --json rust` are both accepted.
- Unknown subcommand and missing-argument errors can be intercepted through `try_parse`.
- Normal clap human output can be preserved when JSON mode is not requested.

Decision target:
- Use clap's global arg support for successful parses.
- Use raw argv scanning only to decide whether parser errors should render as JSON.

### Raw JSON-mode detection

Validate edge cases:
- `bo --json search rust` => JSON mode.
- `bo search --json rust` => JSON mode.
- `bo search rust --json` => JSON mode if clap accepts the global flag there.
- `bo search -- --json` => `--json` is a search term, not JSON mode.
- `bo -- --json` => no subcommand; JSON mode should probably not be inferred after `--`.

Decision target:
- Scan args until `--`; if `--json` appears before that terminator, parser errors render as JSON.

### Parser error command inference

Validate how much command context is available when clap fails.

Possible strategy:
- If first non-flag token is a known command, use it as `command`.
- Otherwise use `"bo"`.

Known commands:
- `seed`
- `collect`
- `compile`
- `list`
- `search`
- `show`
- `raze`

### Compile warning capture

Current compile code prints validation/execution warnings to stderr. Investigate which warnings should become structured top-level JSON warnings.

Candidate structured warnings:
- skipped branch with empty title/body;
- unknown leaf referenced by LLM output;
- branch skipped because it spans fewer than two leaves;
- leaf read/write/frontmatter patch failure during compile;
- skipped leaves from invalid/missing frontmatter.

Decision target:
- Capture warnings that affect final compile result or caller actionability.
- Leave purely diagnostic progress lines on stderr.

### Error typing in `main.rs`

Current top-level command functions often convert errors to `String`. Investigate smallest refactor that preserves typed errors until JSON mapping.

Options:
1. Introduce one `CliError` enum wrapping command-specific errors.
2. Keep command-specific result functions typed and map in each dispatch arm.
3. Use a lightweight error payload directly at the command boundary.

Decision target:
- Avoid broad dependency additions.
- Avoid losing structured details for known errors.

### Process-level tests

Validate whether current test setup can invoke the binary or whether a `run(args, stdout, stderr)` style function is easier.

Decision target:
- Prefer a testable runner function if it avoids brittle subprocess tests.
- Use subprocess tests only for behavior that depends on clap/process exit semantics.

## Libraries or APIs to evaluate

### `clap`

Existing dependency. Areas to check:
- global args in derive API;
- `try_parse_from` / `try_get_matches_from` error handling;
- extracting exit code and rendered message from `clap::Error`;
- preserving default error rendering in non-JSON mode.

### `serde` / `serde_json`

Existing dependencies. Areas to check:
- generic envelope serialization with `Serialize` payloads;
- serializing `serde_yaml_ng::Mapping` inside `show` payload through serde;
- avoiding accidental JSON string double-encoding when wrapping existing renderers.

No new serialization library is expected.

## Performance considerations

JSON serialization cost should be negligible compared with I/O, network collection, and LLM compilation.

Potential considerations:
- `show --json --full` may serialize large leaf bodies. This is expected and mirrors the command request.
- `list --json` serializes all listed rows after filters/limits; existing command already materializes the list.
- `search --json` serializes one page of hits, not the full corpus.
- `compile --json` serializes summary metadata, not full branch bodies.

No special streaming or JSONL support is planned.

## Compatibility considerations

Existing `list`, `search`, and `show` JSON shapes will change because the spec requires a shared envelope. This is acceptable for this feature because the envelope becomes the stable contract going forward.

Human output should remain compatible by default.

## Validation checklist before implementation

- Confirm global `--json` examples work with clap derive.
- Confirm parser errors can produce stdout JSON without clap pre-printing to stderr.
- Confirm non-JSON parser errors still look like normal clap errors.
- Confirm existing serializable result structs do not expose internal-only fields accidentally.
- Confirm search no-results exit code can differ between human and JSON modes without awkward dispatch logic.
- Confirm compile no-op results can be returned without requiring `OPENAI_API_KEY`.
