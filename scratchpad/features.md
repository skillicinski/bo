# Iterate on tests

## `config_path()` still coupled to HOME in other callers

`require_config()` in main.rs still calls `config::config_path()` which reads `HOME`.
Any future command that calls `require_config()` inherits this coupling. The full fix
is threading `config_path: &Path` through `run_cli` so the CLI entry point resolves it
once and passes it down. Do this when the second consumer materializes — it'll need
parametric paths anyway.

## No unit tests for render functions in main.rs

`render_collect_human`, `render_compile_human`, `render_compile_summary_human` are ~100
lines of formatting logic only exercised indirectly through integration tests. Low risk
(pure formatting, no IO) but a coverage gap if formatting logic grows more complex.

## 27 ignored tests are network-dependent

These live in `tests/integration_network.rs` and `tests/integration_compile.rs`. Fine if
CI runs them in a separate step. Problematic if they bitrot silently — consider a
scheduled CI job that runs `cargo test -- --ignored` weekly, or annotate each with a
comment explaining what external service it depends on.
