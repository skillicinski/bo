# Spec: JSON output for CLI commands

## Problem statement

`bo` is useful to humans through concise terminal output, but agents and scripts need stable machine-readable output. Today command output is inconsistent: some commands expose JSON, some do not, and errors/progress/human decorations can require screen-scraping.

This makes `bo` harder to use in agent loops, shell automation, and future MCP-style integrations. A caller should be able to run a command with `--json`, parse stdout as one JSON document, inspect success/failure, and decide the next action without relying on human-formatted text.

## User-facing requirements

- A user can request JSON output with `--json` for every current `bo` command that produces command output:
  - `bo seed --json <output-dir>`
  - `bo collect --json <url>`
  - `bo compile --json`
  - `bo list --json`
  - `bo search --json <term> [<term>...]`
  - `bo show --json <title>`
  - `bo raze --json`
- JSON mode can also be requested before a subcommand is fully parsed, e.g. `bo --json search ...`, so malformed invocations can still produce structured errors.
- Human-readable default output remains unchanged when `--json` is not provided.
- When `--json` is provided, stdout contains exactly one valid JSON object for the command result.
- When `--json` is provided, human decorations such as checkmarks, prose summaries, tables, and progress lines are not printed to stdout.
- Progress messages, diagnostics, logs, and warnings that are not part of the structured result may still be written to stderr.
- JSON is emitted for successful command results, command failures, and command-line usage/parsing failures when JSON mode was requested.
- JSON failures use a nonzero exit code while still writing a parseable JSON error object to stdout.
- Usage/parsing failures include missing required arguments, invalid option values, unknown flags, invalid flag combinations, and unknown subcommands where JSON mode can be detected.
- All JSON responses use a stable top-level envelope:

```json
{
  "schema_version": 1,
  "ok": true,
  "command": "list",
  "data": {},
  "warnings": []
}
```

- Failed responses use the same envelope with `ok: false` and an `error` object:

```json
{
  "schema_version": 1,
  "ok": false,
  "command": "show",
  "error": {
    "code": "not_found",
    "message": "leaf title 'X' not found",
    "details": {}
  },
  "warnings": []
}
```

- The top-level `command` value identifies the command that produced the result. If no subcommand can be determined, `command` is `"bo"`.
- The top-level `schema_version` value allows future compatible evolution of the JSON contract.
- The top-level `warnings` array includes structured warnings and degraded-output notices that may affect how a caller interprets the result.
- JSON field values use normal JSON types directly. The output should not include per-field type descriptors such as `{ "type": "string" }`.
- Output shape should be simple and agent-friendly: predictable keys, shallow structures where practical, required top-level envelope keys, typed scalar values, arrays for repeated items, and enum-like string values for statuses and error codes.
- The JSON contract is stable enough for agents and scripts to treat as an interface. Renaming/removing existing fields should require a schema version change or an explicit compatibility decision.

### Command-specific data requirements

- `seed --json` returns enough information to understand whether a tree was created or already existed, including the configured output directory and tree name when available.
- `collect --json` returns the collected URL and written leaf filename/path. Duplicate, rejected, fetch, extraction, YouTube, and I/O failures return structured error codes and actionable messages.
- `compile --json` returns a status, branches written, leaves updated, leaves skipped, and warnings. Empty-tree and single-leaf cases are successful no-ops with explicit status/reason values.
- `list --json` returns the listed leaves, total index entry count, any branch filter, row degradation status, and row degradation reasons.
- `search --json` returns hits, total result count, page metadata, and the query terms used. Searching an empty tree or a query with no matches is a successful result with an empty hits array.
- `show --json` returns the selected leaf identity, file/path/url, frontmatter, body content returned by the command, and `truncated`/`full` flags. Not-found and ambiguous-title cases return structured errors; ambiguous errors include candidate details when available.
- `raze --json` returns what was removed or left in place, including counts and relevant paths.

## Success criteria

- Every current output-producing command accepts `--json`: `seed`, `collect`, `compile`, `list`, `search`, `show`, and `raze`.
- For each covered command, successful `--json` output is parseable by standard JSON tooling such as `jq`.
- For each covered command, expected application failures under `--json` still emit parseable JSON on stdout and exit nonzero.
- Parser-level usage errors under JSON mode emit parseable JSON on stdout and exit nonzero.
- Under `--json`, stdout contains only the JSON result object and no human progress/decorative output.
- Warnings and degraded states are represented in structured JSON, not only as human text.
- `bo search --json` on an empty tree or with no matches exits successfully and returns an empty hits array.
- `bo compile --json` on an empty tree or one-leaf tree exits successfully and returns an explicit no-op status.
- Existing human-readable command behavior remains available when `--json` is not provided.
- The JSON envelope is consistent across commands and includes `schema_version`, `ok`, `command`, and either `data` or `error`.
- The command-specific `data` payloads include enough information for an agent to take a next action, retry with adjusted input, troubleshoot, or explain the result to a human.

## Out of scope

- Adding new operational commands beyond `seed`, `collect`, `compile`, `list`, `search`, `show`, and `raze`.
- Changing the human-readable default output except where needed to preserve existing behavior around stderr/stdout separation.
- JSON Lines or streaming JSON output.
- A separate schema-discovery command or generated JSON Schema files.
- MCP server implementation or any agent protocol integration.
- Rich nested metadata that is not needed for immediate agent/script actionability.
- Exact internal architecture, Rust module organization, serialization libraries, or error type design.
- CLI help/version output.

## Open questions

None.
