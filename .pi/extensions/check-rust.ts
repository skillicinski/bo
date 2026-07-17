/**
 * Rust Check Extension
 *
 * After every .rs edit/write inside the session project, runs the project's
 * Rust gate (cargo fmt --check, cargo clippy --all-targets --all-features
 * -- -D warnings) from the session cwd. Silent on success; surfaces combined
 * check failures — or a hook-level failure such as cargo being unavailable —
 * as a tool error so the model can react instead of the failure being hidden.
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { isAbsolute, relative, resolve } from "node:path";

const CHECKS: Array<[string, string[]]> = [
	["cargo", ["fmt", "--check"]],
	["cargo", ["clippy", "--all-targets", "--all-features", "--", "-D", "warnings"]],
];

/** True iff `filePath` is a `.rs` file resolved inside the session cwd. */
export function inProject(filePath: string, cwd: string): boolean {
	const normalizedPath = filePath.startsWith("@") ? filePath.slice(1) : filePath;
	if (!normalizedPath.endsWith(".rs")) return false;
	const rel = relative(resolve(cwd), resolve(cwd, normalizedPath));
	return rel !== "" && !rel.startsWith("..") && !isAbsolute(rel);
}

export default function (pi: ExtensionAPI) {
	pi.on("tool_result", async (event, ctx) => {
		if (event.isError || (event.toolName !== "edit" && event.toolName !== "write")) return;

		const input = event.input as { path?: unknown; file_path?: unknown } | undefined;
		const filePath = input?.path ?? input?.file_path;
		const cwd = ctx.cwd;
		if (typeof filePath !== "string" || !inProject(filePath, cwd)) return;

		const errors: string[] = [];
		for (const [cmd, args] of CHECKS) {
			try {
				const { stdout, stderr, code } = await pi.exec(cmd, args, { cwd, signal: ctx.signal });
				if (code !== 0) {
					const output = [stderr, stdout].filter(Boolean).join("\n").trim();
					errors.push(output
						? `${cmd} ${args.join(" ")} failed:\n${output}`
						: `${cmd} ${args.join(" ")} failed with exit code ${code} (no output)`);
				}
			} catch (err) {
				errors.push(`${cmd} ${args.join(" ")} could not run: ${err instanceof Error ? err.message : String(err)}`);
			}
		}

		if (errors.length === 0) return;
		return { isError: true, content: [{ type: "text" as const, text: errors.join("\n\n") }] };
	});
}
