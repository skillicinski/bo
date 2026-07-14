// Run with: node --experimental-strip-types .pi/extensions/test-review-after-pi-commit.mjs
import assert from "node:assert/strict";
import init, { commitWorkingDirectory, isGitCommitCommand } from "./review-after-pi-commit.ts";

assert.equal(isGitCommitCommand('git commit -m "review me"'), true);
assert.equal(isGitCommitCommand("cd repo && git commit -m x"), true);
assert.equal(isGitCommitCommand("git -C repo commit -m x"), true);
assert.equal(isGitCommitCommand("git commit; echo done"), true);
assert.equal(isGitCommitCommand("git commit&& echo done"), true);
assert.equal(isGitCommitCommand("cd /worktree\ngit commit -m x"), true);
assert.equal(isGitCommitCommand("cat <<'EOF'\ngit commit -m x\nEOF"), false);
assert.equal(isGitCommitCommand("cat <<'EOF'\ngit commit -m example\nEOF\ngit commit -m real"), true);
assert.equal(isGitCommitCommand("git push"), false);
assert.equal(isGitCommitCommand("echo 'git commit'"), false);
assert.equal(commitWorkingDirectory("export X=1 && cd /worktree && git commit -m x", "/repo"), "/worktree");
assert.equal(commitWorkingDirectory("cd /tmp && cd /worktree && git commit -m x", "/repo"), "/worktree");
assert.equal(commitWorkingDirectory("cd /worktree\ngit commit -m x", "/repo"), "/worktree");
assert.equal(commitWorkingDirectory("cd /worktree\ngit commit -m x\ncd /other", "/repo"), "/worktree");
assert.equal(
	commitWorkingDirectory(
		"cd /Users/simon/personal-projects/bo.worktrees/issue-190 && git add src/cli/collect.rs && git commit -F /tmp/msg",
		"/Users/simon/personal-projects/bo",
	),
	"/Users/simon/personal-projects/bo.worktrees/issue-190",
);
assert.equal(commitWorkingDirectory("git -C ../worktree commit -m x", "/repo"), "/worktree");

const handlers = new Map();
const emitted = [];
const notifications = [];
let mainHead = "main-base";
let worktreeHead = "worktree-base";
const pi = {
	on(event, handler) {
		handlers.set(event, handler);
	},
	exec(command, args, _options) {
		const key = [command, ...args].join(" ");
		if (key === "git -C /repo rev-parse --show-toplevel") return Promise.resolve({ code: 0, stdout: "/repo\n", stderr: "" });
		if (key === "git -C /worktree rev-parse --show-toplevel") return Promise.resolve({ code: 0, stdout: "/worktree\n", stderr: "" });
		if (key === "git -C /rootrepo rev-parse --show-toplevel") return Promise.resolve({ code: 0, stdout: "/rootrepo\n", stderr: "" });
		if (key === "git -C /repo rev-parse HEAD") return Promise.resolve({ code: 0, stdout: `${mainHead}\n`, stderr: "" });
		if (key === "git -C /worktree rev-parse HEAD") return Promise.resolve({ code: 0, stdout: `${worktreeHead}\n`, stderr: "" });
		if (key === "git -C /rootrepo rev-parse HEAD") return Promise.resolve({ code: 0, stdout: "root-next\n", stderr: "" });
		if (key === "git -C /worktree rev-parse HEAD^") {
			return Promise.resolve({ code: 0, stdout: `${worktreeHead === "branch-next" ? "branch-parent" : "worktree-base"}\n`, stderr: "" });
		}
		if (key === "git -C /rootrepo rev-parse HEAD^") return Promise.resolve({ code: 128, stdout: "", stderr: "root commit" });
		if (key === "git -C /worktree merge-base --is-ancestor worktree-next branch-next") return Promise.resolve({ code: 1, stdout: "", stderr: "not ancestor" });
		if (key === "git -C /worktree show -s --format=%s worktree-next") return Promise.resolve({ code: 0, stdout: "test worktree commit\n", stderr: "" });
		if (key === "git -C /worktree show -s --format=%s branch-next") return Promise.resolve({ code: 0, stdout: "branch switch commit\n", stderr: "" });
		if (key === "git -C /rootrepo show -s --format=%s root-next") return Promise.resolve({ code: 0, stdout: "root commit\n", stderr: "" });
		if (key === "git -C /worktree diff --name-only worktree-base..worktree-next") {
			return Promise.resolve({ code: 0, stdout: "src/lib.rs\ntests/lib.rs\n", stderr: "" });
		}
		if (key === "git -C /worktree diff --name-only branch-parent..branch-next") {
			return Promise.resolve({ code: 0, stdout: "src/branch.rs\n", stderr: "" });
		}
		if (key === "git -C /rootrepo diff-tree --no-commit-id --name-only -r --root root-next") {
			return Promise.resolve({ code: 0, stdout: "Cargo.toml\nsrc/main.rs\n", stderr: "" });
		}
		throw new Error(`unexpected git command: ${key}`);
	},
	events: { emit(channel, data) { emitted.push({ channel, data }); } },
};

init(pi);
const onSessionStart = handlers.get("session_start");
const onToolResult = handlers.get("tool_result");
const onAgentSettled = handlers.get("agent_settled");
const ctx = { cwd: "/repo", signal: undefined, hasUI: true, ui: { notify(message) { notifications.push(message); } } };

await onSessionStart({}, ctx);
worktreeHead = "worktree-next";
await onToolResult({ toolName: "bash", isError: false, input: { command: 'export X=1 && cd /worktree && git commit -m "test"' } }, ctx);
await onToolResult({ toolName: "bash", isError: false, input: { command: 'cd /worktree && git commit -m "second command in same turn"' } }, ctx);
assert.equal(emitted.length, 0, "review waits for agent_settled");
await onAgentSettled({}, ctx);

// Same HEAD after another detected commit is deduplicated.
await onToolResult({ toolName: "bash", isError: false, input: { command: 'cd /worktree && git commit -m "already reviewed"' } }, ctx);
await onAgentSettled({}, ctx);

// A branch switch falls back to the immediate parent instead of diffing from stale worktree-next.
worktreeHead = "branch-next";
await onToolResult({ toolName: "bash", isError: false, input: { command: 'cd /worktree && git commit -m "branch"' } }, ctx);
await onAgentSettled({}, ctx);

// Root commit takes the --root diff-tree branch.
await onToolResult({ toolName: "bash", isError: false, input: { command: 'cd /rootrepo && git commit -m "root"' } }, ctx);
await onAgentSettled({}, ctx);
await onToolResult({ toolName: "bash", isError: true, input: { command: 'git commit -m "failed"' } }, ctx);
await onAgentSettled({}, ctx);

assert.deepEqual(emitted, [
	{
		channel: "delegate:review-request",
		data: {
			mode: "fast",
			cwd: "/worktree",
			commit: "worktree-next",
			subject: "test worktree commit",
			changedFiles: ["src/lib.rs", "tests/lib.rs"],
			source: "successful Pi git commit",
		},
	},
	{
		channel: "delegate:review-request",
		data: {
			mode: "fast",
			cwd: "/worktree",
			commit: "branch-next",
			subject: "branch switch commit",
			changedFiles: ["src/branch.rs"],
			source: "successful Pi git commit",
		},
	},
	{
		channel: "delegate:review-request",
		data: {
			mode: "fast",
			cwd: "/rootrepo",
			commit: "root-next",
			subject: "root commit",
			changedFiles: ["Cargo.toml", "src/main.rs"],
			source: "successful Pi git commit",
		},
	},
]);
assert.deepEqual(notifications, ["Fast review queued for worktree", "Fast review queued for branch-n", "Fast review queued for root-nex"]);

console.log("review-after-pi-commit checks: OK");
