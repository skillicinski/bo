# Development Rules

## Conversational Style

- Keep answers short and concise
- No emojis in commits, issues, PR comments, or code
- No fluff or cheerful filler text (e.g., "Thanks @user" not "Thanks so much @user!")
- Technical prose only, be direct
- When the user asks a question, answer it first before making edits or running implementation commands.
- When responding to user feedback or an analysis, explicitly say whether you agree or disagree before saying what you changed.

## Code Quality

- Read files in full before wide-ranging changes, before editing files you have not fully inspected, and when asked to investigate or audit. Do not rely on search snippets for broad changes.
- Inline single-line helpers that have only one call site.
- Always ask before removing functionality or code that appears intentional.
- Do not preserve backward compatibility unless the user asks for it.

## Commands

- After code changes (not docs), run `cargo fmt --check` and `cargo clippy --all-targets --all-features -- -D warnings` with full output. Fix all errors and warnings before committing.
- Before opening a PR, also run `cargo test` as required by `CONTRIBUTING.md`.

## Dependency and Install Security

- Treat npm dep and lockfile changes as reviewed code. Direct external deps stay pinned to exact versions.
- Hydrate/update locally with npm install --ignore-scripts; clean/CI-style with npm ci --ignore-scripts. Don't run lifecycle scripts unless the user asks.
- If dep metadata changes, refresh package-lock.json with npm install --package-lock-only --ignore-scripts.

## Git

Multiple pi sessions may be running in this cwd at the same time, each modifying different files. Git operations that touch unstaged, staged, or untracked files outside your own changes will stomp on other sessions' work. Follow these rules:

Committing:

- Only commit files YOU changed in THIS session.
- Stage explicit paths (git add <path1> <path2>); never git add -A / git add ..
- Before committing, run git status and verify you are only staging your files.
packages/ai/src/models.generated.ts may always be included alongside your files.
- Message format: {feat,fix,refactor,docs,test}[(ai,tui,agent,coding-agent)]: <commit message> (optionally multiple lines). Message is informative and concise.

Never run (destroys other agents' work or bypasses checks):
- git reset --hard, git checkout ., git clean -fd, git stash, git add -A, git add ., git commit --no-verify.

If rebase conflicts occur:

- Resolve conflicts only in files you modified.
- If a conflict is in a file you did not modify, abort and ask the user.
- Never force push.

## Issues & PRs

See CONTRIBUTING.md for the contributor gate.

### Solving issues

- Read the issue body and comments, linked or blocking issues, canonical project documentation, and the milestone description when the issue is assigned to one. Confirm required predecessor work has merged.
- Revalidate the issue against current `main`. Line numbers, proposed module boundaries, and implementation sketches are evidence, not immutable specifications.
- If the premise is stale, duplicated, blocked, or conflicts with current architecture, update or close the issue before changing code.
- Treat acceptance criteria and existing observable behavior as the contract unless the issue explicitly permits a breaking change. Do not add unspecified backward compatibility.
- Prefer the smallest complete change. Do not expand scope or introduce abstractions without a concrete need in the current issue.
- Do not reopen settled architectural decisions without new concrete evidence.
- An issue is not solved when implementation ends. After required checks pass, audit the completed diff against the issue's acceptance criteria before declaring it done.
- For non-trivial completed work, delegate a read-only review after the builder finishes, never in the same parallel batch:
  - use `reviewer` for the default focused review;
  - use `reviewer-ensemble` for significant multi-file changes needing independent cross-vendor review;
  - use `adversarial-reviewer` for architecture, security, state consistency, migration, or other cross-cutting risks.
- Treat delegated findings as review assertions until their evidence is verified. Do not discard an evidence-backed finding by majority vote.
- Review findings cite `file:line` evidence and are classified as blockers, requested changes, or notes.

### Pull requests

When posting issue/PR comments:

- Write the comment to a temp file and post with gh issue/pr comment --body-file (never multi-line markdown via --body).
- Keep comments concise, technical, in the user's tone.

When closing issues via commit:

- Include fixes #<number> or closes #<number> in the message so merging auto-closes the issue. For multiple issues, repeat the keyword per issue (closes #1, closes #2); a shared keyword (closes #1, #2) only closes the first.

When reviewing PRs:

- Do not run gh pr checkout, git switch, or otherwise move the worktree to the PR branch unless the user explicitly asks.
- Use gh pr view, gh pr diff, gh api, and local git show/git diff against fetched refs to inspect PR metadata, commits, and patches without changing branches.
- If you need PR file contents, fetch/read them into temporary files or use git show <ref>:<path> without switching branches.

When creating PRs:

- PRs don't need "Verification", "Tests", or "How to test" sections. CI gates every merge and PRs go through human review. A brief summary of what changed and why is sufficient.
- When creating new PRs, the body should begin with a itemised list that describes the core functional changes to the project. Items are formatted use the unordered list markdown syntax. Each item should begin with a null-subject verb, eg. "added", "removed", "updated", "refactored", "bumped" to describe the type of operation on the code, followed by a one-line summary of the change itself.
- Link related issues and stacked PRs at the bottom of the PR body, separated from the summary by `---` on its own line:
  - `Closes #<number>` for issues this PR resolves
  - `Stacked on #<number>` for an open PR this builds on top of

## Changelog

`CHANGELOG.md` follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). When adding a release entry:

- Add a new `## [x.y.z] - YYYY-MM-DD` section at the top (below the header).
- Group changes under `### Added`, `### Changed`, `### Fixed`, `### Removed` as applicable.
- Write entries from the user's perspective, not implementation details.
- Keep entries concise — one line per change.

## Releasing

This project follows [Semantic Versioning](https://semver.org/).

1. Update `CHANGELOG.md` with the new version section.
2. Local smoke test: build an unpublished release and smoke test from outside the repo (so it can't resolve workspace files):
3. Bump `version` in `Cargo.toml`.
4. Bump `version` in `npm/package.json` to match.
5. Commit, merge to main.
6. `git tag v<version> && git push --tags`
7. **Approve the `npm-publish` GitHub Environment** — Actions tab → tag run → click *Review deployments* → approve. The job will not run until you do.
8. **Approve the staged tarball on npmjs.com** — `npm stage publish` uploads to a staging queue, not to public. Open the package page on npmjs.com from a 2FA-trusted device and approve the pending stage.

The `release.yml` workflow runs CI (format, clippy, deny, test), builds platform binaries for macOS Intel/Apple Silicon and Linux x86_64, creates a GitHub Release with the tarballs attached, and stages `@skillicinski/bo` to npm via OIDC trusted publishing.

### Why two human gates?

The environment approval (step 6) blocks token issuance: the OIDC token GitHub mints carries the `environment: npm-publish` claim, which npm's trusted-publisher config requires. The staged-publish approval (step 7) blocks public availability: the tarball is uploaded but invisible until a maintainer signs off from a trusted device. Either gate alone would not stop a compromised CI from publishing.

### Recovering from a failed release

- If `npm-publish` failed (e.g. before this hardening), delete the partial release and re-tag at a fixed commit:
  ```bash
  gh release delete v<version> --yes --cleanup-tag
  git tag -d v<version>
  git tag v<version> <fixed-commit>
  git push origin v<version>
  ```
- Versions that successfully publish to npm are **permanently burned** even after `npm unpublish`. Bump to the next patch instead of reusing.
- Re-running a tag-triggered workflow replays the workflow YAML from the tag's commit, not from `main`. If the fix is on `main`, you must re-tag, not just re-run.

## User Override

If the user's instructions conflict with any rule in this document, ask for explicit confirmation before overriding. Only then execute their instructions.
