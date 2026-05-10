# Tasks: JSON output for CLI commands

## Prerequisites and validation

- [x] Validate clap global flag behavior for `--json` with derive subcommands, including `bo --json search rust` and `bo search --json rust`.
- [x] Validate raw argv JSON-mode detection rules, especially stopping at `--` so `bo search -- --json` treats `--json` as an argument.
- [x] Add or identify a testable CLI runner boundary, e.g. `run_from(args, stdout, stderr) -> exit_code`, so parser errors, stdout/stderr, and exit codes can be tested without brittle subprocess tests.

## Shared JSON response infrastructure

- [x] Add shared JSON envelope types for success responses with `schema_version`, `ok`, `command`, `data`, and `warnings`.
- [x] Add shared JSON envelope types for error responses with `schema_version`, `ok`, `command`, `error`, and `warnings`.
- [x] Add shared `JsonError` shape with `code`, `message`, and `details` fields.
- [x] Add shared `JsonWarning` shape with `code`, `message`, and `details` fields.
- [x] Add helper functions for serializing success and error envelopes to exactly one JSON object.
- [x] Add unit tests proving success and error envelopes serialize with the required top-level fields and no double-encoded payloads.

## CLI parsing and dispatch

- [x] Add a global `--json` CLI flag accepted before or after supported subcommands.
- [x] Replace direct clap process-exiting parsing with error-aware parsing that can render parser errors as JSON.
- [x] Preserve normal clap human error output when JSON mode was not requested.
- [x] Emit JSON error envelopes on stdout for parser-level errors when JSON mode was requested.
- [x] Infer the `command` field for parser errors from raw args when possible, falling back to `"bo"`.
- [x] Ensure JSON parser errors exit nonzero and human parser errors keep normal nonzero behavior.

## Error mapping

- [x] Add a CLI-facing error mapping layer from known application errors to stable JSON error codes.
- [x] Map missing config / not-seeded failures to `not_seeded` with an actionable message.
- [x] Map collect duplicate URL failures to `duplicate_url` with `existing_file` details.
- [x] Map collect rejected-content failures to `rejected` with URL and reason details.
- [x] Map collect fetch, extract, YouTube, and I/O failures to distinct structured error codes where possible.
- [x] Map show not-found failures to `not_found`.
- [x] Map show ambiguous-title failures to `ambiguous` with candidate details.
- [x] Map compile context overflow, truncated output, content filter, LLM, validation, and I/O failures to distinct structured error codes.
- [x] Add a fallback `unknown_error` mapping for unexpected string/opaque errors.

## `list` JSON behavior

- [x] Wrap `bo list --json` output in the shared success envelope instead of emitting the raw list payload.
- [x] Preserve `bo list` human output when `--json` is absent.
- [x] Include list rows, total index entries, branch filter, degradation status, and degradation reasons in `data`.
- [x] Add top-level JSON warnings for degraded list rows where useful without removing per-row degradation fields.
- [x] Add tests for `bo list --json`, including empty trees, branch filters, limits, and degraded rows.

## `search` JSON behavior

- [x] Wrap `bo search --json` output in the shared success envelope instead of emitting the raw search payload.
- [x] Add query terms to the `search` JSON `data` payload.
- [x] Preserve `bo search` human output and no-results exit behavior when `--json` is absent.
- [x] Change JSON-mode no-results behavior to `ok: true`, `hits: []`, and exit code `0`.
- [x] Add tests for successful search JSON, no-result search JSON, pagination metadata, and query term payload.

## `show` JSON behavior

- [x] Wrap `bo show --json` output in the shared success envelope instead of emitting the raw show payload.
- [x] Preserve `bo show` human output when `--json` is absent.
- [x] Include selected leaf identity, file, path, URL, frontmatter, body, `truncated`, and `full` fields in `data.leaf`.
- [x] Emit JSON error envelopes for not-found, ambiguous, missing-file, unreadable-file, suspicious-path, and invalid-frontmatter failures.
- [x] Add tests for `bo show --json`, `bo show --json --full`, not-found JSON errors, and ambiguous-title JSON errors.

## `seed` JSON behavior

- [x] Refactor seed execution to return a typed seed result instead of printing directly from command logic.
- [x] Add `bo seed --json <output-dir>` support with status, output directory, and tree name in `data`.
- [x] Represent already-seeded behavior as a successful JSON status if human mode currently treats it as successful.
- [x] Preserve existing `bo seed` human output when `--json` is absent.
- [x] Add tests for created and already-seeded JSON outcomes.

## `collect` JSON behavior

- [x] Refactor collect command wrapper to return a typed collect result and preserve typed collect errors until JSON mapping.
- [x] Add `bo collect --json <url>` support with collected URL, relative file, and full path in `data`.
- [x] Ensure progress such as fetching/summarizing does not appear on stdout in JSON mode.
- [x] Preserve existing `bo collect` human output and stderr progress when `--json` is absent.
- [x] Add tests for successful collect JSON using fixture/local collect paths where possible.
- [x] Add tests for duplicate URL JSON errors.

## `raze` JSON behavior

- [x] Refactor raze execution to return a typed deletion summary instead of printing directly from command logic.
- [x] Add `bo raze --json` support with deleted file count, index deletion status, output directory status, config deletion status, and relevant paths.
- [x] Represent skipped suspicious ledger entries as structured warnings.
- [x] Preserve existing `bo raze` human output when `--json` is absent.
- [x] Add tests for raze JSON summary and suspicious-entry warnings.

## `compile` JSON behavior

- [x] Refactor compile pipeline to return a typed compile result instead of printing summary/no-op messages internally.
- [x] Add compile status values `compiled` and `noop`.
- [x] Return `ok: true` JSON no-op result with reason `empty_tree` for empty trees.
- [x] Return `ok: true` JSON no-op result with reason `single_leaf` for one-leaf trees.
- [x] Include branches written, leaves updated, and leaves skipped in `compile` JSON `data`.
- [x] Preserve existing `bo compile` human summary/no-op output when `--json` is absent.
- [x] Capture actionable compile warnings structurally where practical while keeping diagnostics/progress on stderr.
- [x] Add tests for empty-tree JSON no-op, single-leaf JSON no-op, missing API key JSON error, and human-mode no-op preservation.

## Cross-command JSON contract tests

- [x] Add tests proving every current output-producing command accepts `--json`: `seed`, `collect`, `compile`, `list`, `search`, `show`, and `raze`.
- [x] Add tests proving `bo --json <command>` works for representative commands.
- [x] Add tests proving JSON-mode stdout contains exactly one parseable JSON object and no human decorations.
- [x] Add tests proving JSON-mode command failures write parseable JSON to stdout and exit nonzero.
- [x] Add tests proving parser-level usage errors under JSON mode write parseable JSON to stdout and exit nonzero.
- [x] Add tests proving top-level envelopes always include `schema_version`, `ok`, `command`, and `warnings`, plus exactly one of `data` or `error`.

## Final verification

- [x] Run `cargo fmt --check` and fix formatting issues.
- [x] Run `cargo clippy --all-targets --all-features -- -D warnings` and fix warnings.
- [x] Run `cargo test` and fix failures.
- [x] Manually smoke-test representative commands in both human and JSON modes.
