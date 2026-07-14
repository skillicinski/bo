/**
 * Queue one fast Delegate review after successful `git commit` commands issued through Pi.
 *
 * This is deliberately a Pi-session hook, not a Git post-commit hook: terminal,
 * IDE, and CI commits are outside this process and are not observed.
 */

import { resolve } from "node:path";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";

// Supports direct, `cd <worktree> &&`, and `git -C <worktree>` commit forms.
// This intentionally is not a general shell parser (no env/-c wrapper support).
const GIT_COMMIT = /(?:^|[;&|\n]\s*)\bgit(?:\s+-C\s+(?:"[^"]+"|'[^']+'|\S+))*\s+commit(?:\s|[;&|\n]|$)/;
const LEADING_CD = /(?:^|[;&|\n]\s*)cd\s+(?:"([^"]+)"|'([^']+)'|(\S+))\s*(?=&&|;|\||\n|$)/g;
const GIT_C = /\bgit\s+-C\s+(?:"([^"]+)"|'([^']+)'|(\S+))/g;

/** Remove simple heredoc bodies so embedded shell examples do not count as executed commands. */
function withoutHeredocBodies(command: string): string {
	let terminator: string | undefined;
	return command
		.split("\n")
		.filter((line) => {
			if (terminator) {
				if (line.trim() === terminator) terminator = undefined;
				return false;
			}
			const heredoc = line.match(/<<-?\s*(['"]?)([A-Za-z_][A-Za-z0-9_]*)\1/);
			if (heredoc) terminator = heredoc[2];
			return true;
		})
		.join("\n");
}

export function isGitCommitCommand(command: string): boolean {
	return GIT_COMMIT.test(withoutHeredocBodies(command));
}

/** Resolve the repo target used by common `cd <dir> && git commit` and `git -C <dir> commit` forms. */
export function commitWorkingDirectory(command: string, cwd: string): string {
	const shell = withoutHeredocBodies(command);
	const commitIndex = shell.search(GIT_COMMIT);
	const matches = [...shell.matchAll(LEADING_CD), ...shell.matchAll(GIT_C)]
		.filter((match) => (match.index ?? 0) <= commitIndex)
		.sort((a, b) => (a.index ?? 0) - (b.index ?? 0));
	const match = matches.at(-1);
	const target = match?.[1] ?? match?.[2] ?? match?.[3];
	return target ? resolve(cwd, target) : cwd;
}

export default function (pi: ExtensionAPI) {
	const reviewedHeads = new Map<string, string>();
	const pendingRoots = new Set<string>();

	const repoRoot = async (cwd: string, signal?: AbortSignal): Promise<string | undefined> => {
		const result = await pi.exec("git", ["-C", cwd, "rev-parse", "--show-toplevel"], { signal });
		return result.code === 0 ? result.stdout.trim() || undefined : undefined;
	};

	const repoHead = async (root: string, signal?: AbortSignal): Promise<string | undefined> => {
		const result = await pi.exec("git", ["-C", root, "rev-parse", "HEAD"], { signal });
		return result.code === 0 ? result.stdout.trim() || undefined : undefined;
	};

	pi.on("session_start", async (_event, ctx) => {
		try {
			const root = await repoRoot(ctx.cwd);
			if (!root) return;
			const head = await repoHead(root);
			if (head) reviewedHeads.set(root, head);
		} catch {
			// Not a Git repo or git unavailable: post-commit review stays inert.
		}
	});

	pi.on("tool_result", async (event, ctx) => {
		if (event.toolName !== "bash" || event.isError) return;
		const command = (event.input as { command?: unknown }).command;
		if (typeof command !== "string" || !isGitCommitCommand(command)) return;

		try {
			const root = await repoRoot(commitWorkingDirectory(command, ctx.cwd), ctx.signal);
			if (root) pendingRoots.add(root);
		} catch {
			// A failed metadata query must never affect the completed commit.
		}
	});

	pi.on("agent_settled", async (_event, ctx) => {
		const roots = [...pendingRoots];
		pendingRoots.clear();

		for (const root of roots) {
			try {
				const commit = await repoHead(root, ctx.signal);
				if (!commit || commit === reviewedHeads.get(root)) continue;

				let base = reviewedHeads.get(root);
				if (base) {
					const ancestor = await pi.exec("git", ["-C", root, "merge-base", "--is-ancestor", base, commit], { signal: ctx.signal });
					if (ancestor.code !== 0) base = undefined;
				}
				if (!base) {
					const parent = await pi.exec("git", ["-C", root, "rev-parse", "HEAD^"], { signal: ctx.signal });
					base = parent.code === 0 ? parent.stdout.trim() : undefined;
				}
				const [subject, changed] = await Promise.all([
					pi.exec("git", ["-C", root, "show", "-s", "--format=%s", commit], { signal: ctx.signal }),
					base
						? pi.exec("git", ["-C", root, "diff", "--name-only", `${base}..${commit}`], { signal: ctx.signal })
						: pi.exec("git", ["-C", root, "diff-tree", "--no-commit-id", "--name-only", "-r", "--root", commit], { signal: ctx.signal }),
				]);
				if (subject.code !== 0 || changed.code !== 0) continue;

				reviewedHeads.set(root, commit);
				pi.events.emit("delegate:review-request", {
					mode: "fast",
					cwd: root,
					commit,
					subject: subject.stdout.trim(),
					changedFiles: changed.stdout.split("\n").filter(Boolean),
					source: "successful Pi git commit",
				});
				if (ctx.hasUI) ctx.ui.notify(`Fast review queued for ${commit.slice(0, 8)}`, "info");
			} catch {
				// A failed metadata query must never affect the completed commit.
			}
		}
	});
}
