// Run with: node --experimental-strip-types .pi/extensions/test-check-rust.mjs
import assert from "node:assert/strict";
import init, { inProject } from "./check-rust.ts";

assert.equal(inProject("src/lib.rs", "/repo"), true);
assert.equal(inProject("/repo/src/lib.rs", "/repo"), true);
assert.equal(inProject("src/sub/mod.rs", "/repo"), true);
assert.equal(inProject("@src/lib.rs", "/repo"), true); // Pi strips this prefix for built-in tools
assert.equal(inProject("../other/src/lib.rs", "/repo"), false);
assert.equal(inProject("/repo-evil/src/lib.rs", "/repo"), false); // loose-prefix trap
assert.equal(inProject("/tmp/x.rs", "/repo"), false);
assert.equal(inProject("src/lib.txt", "/repo"), false);
assert.equal(inProject("/repo/Cargo.toml", "/repo"), false);

const ctx = { cwd: "/repo", signal: undefined };

function makePi(responses) {
	const calls = [];
	return {
		calls,
		on(_event, handler) {
			this.handler = handler;
		},
		async exec(cmd, args, opts) {
			calls.push({ cmd, args, opts });
			const key = `${cmd} ${args.join(" ")}`;
			if (key in responses) {
				const r = responses[key];
				if (r.throw) throw new Error(r.throw);
				return { code: r.code, stdout: r.stdout ?? "", stderr: r.stderr ?? "" };
			}
			throw new Error(`unexpected exec: ${key}`);
		},
	};
}

const run = (pi, input, toolName = "edit") => {
	init(pi);
	return pi.handler({ toolName, isError: false, input }, ctx);
};
const CLIPPY = "cargo clippy --all-targets --all-features -- -D warnings";
const FMT = "cargo fmt --check";

// Non-.rs file: no patch, no checks.
{
	const pi = makePi({ [FMT]: { code: 0 } });
	assert.equal(await run(pi, { path: "README.md" }), undefined);
	assert.equal(pi.calls.length, 0);
}

// .rs outside the project: guard skips, no patch, no checks.
{
	const pi = makePi({ [FMT]: { code: 0 } });
	assert.equal(await run(pi, { path: "/tmp/x.rs" }), undefined);
	assert.equal(pi.calls.length, 0);
}

// All pass: silent, exact gate invoked from the session cwd.
{
	const pi = makePi({ [FMT]: { code: 0 }, [CLIPPY]: { code: 0 } });
	assert.equal(await run(pi, { path: "src/lib.rs" }), undefined);
	assert.deepEqual(
		pi.calls.map((c) => `${c.cmd} ${c.args.join(" ")}`),
		[FMT, CLIPPY],
	);
	for (const c of pi.calls) assert.equal(c.opts.cwd, "/repo");
}

// Legacy file_path and @ prefix resolve like Pi's built-in write tool.
{
	const pi = makePi({ [FMT]: { code: 0 }, [CLIPPY]: { code: 0 } });
	assert.equal(await run(pi, { file_path: "@src/lib.rs" }, "write"), undefined);
	assert.equal(pi.calls.length, 2);
}

// Clippy fails: surface both streams in full.
{
	const pi = makePi({ [FMT]: { code: 0 }, [CLIPPY]: { code: 101, stdout: "note: more details\n", stderr: "warning: unused variable\n" } });
	const out = await run(pi, { path: "src/lib.rs" });
	assert.equal(out.isError, true);
	assert.match(out.content[0].text, /clippy --all-targets --all-features -- -D warnings failed/);
	assert.match(out.content[0].text, /unused variable/);
	assert.match(out.content[0].text, /more details/);
}

// A zero-output failure still reports the exit status.
{
	const pi = makePi({ [FMT]: { code: 1 }, [CLIPPY]: { code: 0 } });
	const out = await run(pi, { path: "src/lib.rs" });
	assert.match(out.content[0].text, /exit code 1 \(no output\)/);
}

// Hook-level failure (exec throws): surfaced, not silently swallowed.
{
	const pi = makePi({ [FMT]: { code: 0 }, [CLIPPY]: { throw: "ENOENT cargo" } });
	const out = await run(pi, { path: "src/lib.rs" });
	assert.equal(out.isError, true);
	assert.match(out.content[0].text, /could not run/);
}

console.log("check-rust checks: OK");
