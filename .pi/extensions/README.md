# Pi Extensions — bo

## check-rust.ts

Runs `cargo check`, `cargo fmt --check`, and `cargo clippy -- -D warnings` after every `.rs` file modification (edit or write). Auto-loads when pi is launched from the repo root. Silent on success; on failure, surfaces the combined cargo errors as a tool error so the model can self-correct.
