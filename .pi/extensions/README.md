# Pi Extensions — bo

## check-rust.ts

Runs the project Rust gate — `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings` (per `AGENTS.md`/`CONTRIBUTING.md`) — after every `.rs` edit or write that lands inside the session cwd. Checks run from the session cwd, so edits in worktrees target the right project and edits outside the project are skipped. Silent on success; on failure it surfaces the combined cargo output as a tool error so the model can self-correct. A hook-level failure (e.g. cargo unavailable) is surfaced rather than swallowed.

Run its no-network regression check with:

```bash
node --experimental-strip-types .pi/extensions/test-check-rust.mjs
```

## review-after-pi-commit.ts

Observes successful `git commit` commands issued through Pi's `bash` tool. After the agent settles, it resolves the command's target worktree (including `cd <worktree> && git commit` and `git -C <worktree> commit`), then collects final `HEAD`, subject, and paths changed since that worktree's last review baseline. It emits `delegate:review-request` with `mode: "fast"`; the global Delegate extension queues a read-only `reviewer-fast` follow-up.

This is deliberately not a Git hook: commits made from Terminal, an IDE, CI, or another Pi session are not observed. It does not block commits. Supported command forms are direct `git commit`, `cd <worktree> && git commit`, and `git -C <worktree> commit`; wrapper flags between `git` and `commit` are intentionally out of scope.

Run its no-network regression check with:

```bash
node --experimental-strip-types .pi/extensions/test-review-after-pi-commit.mjs
```
