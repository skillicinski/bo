/**
 * Rust Check Extension
 *
 * Runs cargo check, fmt, and clippy after every .rs file edit/write.
 * Silent on success; surfaces errors as tool errors.
 */

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import * as path from "node:path";

export default function (pi: ExtensionAPI) {
	pi.on("tool_result", async (event) => {
		if (event.toolName !== "edit" && event.toolName !== "write") return;

		const filePath = (event.input as { path?: string } | undefined)?.path;
		if (!filePath || !filePath.endsWith(".rs")) return;

		const projectRoot = process.cwd();
		const abs = path.resolve(filePath);
		if (!abs.startsWith(projectRoot)) return;

		const errors: string[] = [];
		const checks: Array<[string, string[]]> = [
			["cargo", ["check", "--quiet"]],
			["cargo", ["fmt", "--check", "--quiet"]],
			["cargo", ["clippy", "--quiet", "--", "-D", "warnings"]],
		];

		for (const [cmd, args] of checks) {
			const { stdout, stderr, code } = await pi.exec(cmd, args);
			if (code !== 0) {
				errors.push(`${cmd} ${args.join(" ")} failed:\n${stderr || stdout}`);
			}
		}

		if (errors.length === 0) return;

		return {
			isError: true,
			content: [{ type: "text" as const, text: errors.join("\n\n") }],
		};
	});
}
