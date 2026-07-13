# Source architecture instructions

`docs/architecture.md` is the sole source of truth for layer direction, ownership, visibility, and pipeline structure. Read and follow it before changing `src/`.

Do not restate the dependency graph here. `tests/architecture.rs` is its executable backstop; update the document and test together when the policy changes.
