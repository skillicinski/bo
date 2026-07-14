# Pi Extensions — bo

## check-rust.ts

Runs `cargo check`, `cargo fmt --check`, and `cargo clippy -- -D warnings` after every `.rs` file modification (edit or write). Auto-loads when pi is launched from the repo root. Silent on success; on failure, surfaces the combined cargo errors as a tool error so the model can self-correct.

## review-after-pi-commit.ts

Observes successful `git commit` commands issued through Pi's `bash` tool. After the agent settles, it resolves the command's target worktree (including `cd <worktree> && git commit` and `git -C <worktree> commit`), then collects final `HEAD`, subject, and paths changed since that worktree's last review baseline. It emits `delegate:review-request` with `mode: "fast"`; the global Delegate extension queues a read-only `reviewer-fast` follow-up.

This is deliberately not a Git hook: commits made from Terminal, an IDE, CI, or another Pi session are not observed. It does not block commits. Supported command forms are direct `git commit`, `cd <worktree> && git commit`, and `git -C <worktree> commit`; wrapper flags between `git` and `commit` are intentionally out of scope.

Run its no-network regression check with:

```bash
node --experimental-strip-types .pi/extensions/test-review-after-pi-commit.mjs
```
