# Analysis: Organise src/ by architectural layers

## Risk assessment

**Low overall risk** — this is a mechanical refactor with no behavioural changes. The compiler is the verification tool.

1. **Test relocation volume** — `tests/integration.rs` is ~280 lines (tests + HTML fixtures) moving into `cli/collect.rs` (currently 289 lines). The resulting file will be ~570 lines. Acceptable for a module with inline tests, but close to the threshold where splitting tests into a submodule (`cli/collect/tests.rs`) would help. Not blocking — note for future.

2. **`pub use quality::RejectReason` at crate root** — Currently lib.rs re-exports this. The plan says "no crate-root re-exports" so this goes away. Only `collect.rs` uses it internally (via `crate::RejectReason`), which will become `crate::engine::quality::RejectReason`. No external consumer uses `bo::RejectReason`. Safe to drop.

3. **Visibility of `quality` module** — Currently `mod quality` (private) with a single `pub use`. After the move it becomes `pub mod quality` inside `engine/mod.rs`. This widens visibility slightly but has no practical impact (binary crate, no external consumers).

## Gap analysis

1. **`compile.rs` test env manipulation** — The offline compile tests call `std::env::remove_var("OPENAI_API_KEY")`. When moved in-module, they share a process with other tests. This already exists today (they run in `cargo test` alongside everything else) so it's not a new problem, but worth noting — these tests should use a serial mutex if they ever conflict.

2. **Doc comment for new lib.rs** — The current lib.rs has extensive module-dependency documentation. The plan specifies a minimal 4-line rewrite. Recommend adding a brief doc comment explaining the layer architecture (5–10 lines). Not blocking.

3. **`adapters/` imports** — `adapters/` uses only `super::` internally and has no `crate::` imports. Confirmed no changes needed there.

## Edge cases

1. **`TreeConfig` serde derives** — Moving `TreeConfig` to `domain/tree.rs` means domain depends on `serde::{Serialize, Deserialize}`. This doesn't violate the layering rule (serde is an external crate, not an internal layer), but it's worth noting that "domain has zero I/O" doesn't mean "domain has zero external dependencies."

2. **`Config` struct references `TreeConfig`** — After the move, `engine/config.rs` contains `Config { pub tree: TreeConfig, ... }` where `TreeConfig` is imported from `crate::domain::tree`. The `Serialize`/`Deserialize` derives on `Config` require that `TreeConfig` also derives them — which it already does. No issue.

3. **Integration test fixture helpers use library API** — `tests/integration_cli.rs` uses `bo::index::append_entry` for fixture setup. This is acceptable — it's a test helper reaching into the library for convenience, not testing that function's behaviour. The actual assertions go through the binary.

## Dependencies

None. Pure internal refactor with no external blockers.

## Recommendation

**Ready to implement.** All gaps have been addressed in the updated spec and tasks. The work is mechanical and the compiler will catch any missed paths immediately.
