# bo

`bo` is a local-first knowledge workspace. It stores source snapshots as
Markdown, keeps workspace state on disk, and can synthesize summaries or one
cross-source distill document with an LLM. No server or database is required.

## Install

Install a pre-built binary with npm:

```bash
npm install -g @skillicinski/bo
```

Install the latest CLI from source with Go:

```bash
go install github.com/skillicinski/bo/cmd/bo@latest
```

The CLI stores local workspaces under `$HOME/.bo`. Set `HOME` on Unix or
`USERPROFILE` on Windows when another location is required.

## CLI workflows

The CLI has five workflows. Each command works with one named local workspace.

### 1. Seed a workspace

```bash
bo seed --name notes
# seeded: notes
```

Without `--name`, bo generates a name such as `quiet-wren`.

### 2. Snap source content

Snap a web page or a local Markdown file. The source is stored as a raw
snapshot in the workspace.

```bash
bo snap notes https://example.com/article
bo snap notes ./note.md
```

### 3. Read workspace state

```bash
bo state notes
bo state notes --full
```

The short form prints the snapshot count. `--full` prints the state JSON.

### 4. Synthesize summaries

Synthesis needs a DeepSeek API key. It reads the newest raw snapshot for each
source and writes summaries back to the same workspace.

```bash
export DEEPSEEK_API_KEY=...
bo synth notes
```

Use runtime limits such as `--max-turns`, `--max-tool-calls`, and
`--timeout-seconds` when required. The timeout applies to each agent runtime;
the caller context controls the complete workflow.

### 5. Distill across sources

Distill selects one useful theme supported by at least two source identities.
It reads current raw snapshots and current summaries, then creates at most one
new Markdown document under `distillations/`. It never changes summaries.

```bash
bo distill notes
```

The command reports the created filename or why it skipped the write. It uses
the same runtime limit flags as `bo synth`.

## Go library

Import `github.com/skillicinski/bo` when an application needs workflow results
instead of CLI output. A library caller follows the same workflow as the CLI:

1. create a workspace manager;
2. seed it if needed;
3. open the workspace;
4. call `Snap`, `ReadState`, `Synth`, or `Distill`;
5. handle the result and close the workspace.

`Seed` creates a workspace, `Snap` reads source content and stores a raw
snapshot, `ReadState` returns the inventory, `Synth` writes summaries, and
`Distill` writes one cross-source document.

The local adapter is available through `LocalManager`:

```go
package main

import (
	"context"
	"os"

	bo "github.com/skillicinski/bo"
)

func run() error {
	ctx := context.Background()
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}

	manager := bo.NewLocalManager(home)
	if _, err := bo.Seed(ctx, bo.SeedRequest{
		Creator: manager,
		Name:    "notes",
	}); err != nil && !bo.IsAlreadyExists(err) {
		return err
	}

	workspace, err := manager.Open(ctx, "notes")
	if err != nil {
		return err
	}
	defer workspace.Close()

	result, err := bo.Snap(ctx, bo.SnapRequest{
		Workspace: workspace,
		Sources:   []string{"./note.md"},
	})
	if err != nil {
		return err
	}
	for _, outcome := range result.Outcomes {
		if outcome.Err != nil {
			return outcome.Err
		}
	}
	return nil
}

func main() {
	if err := run(); err != nil {
		panic(err)
	}
}
```

Applications can provide another backend by implementing `bo.Workspace` and
passing it to `Snap`, `ReadState`, `Synth`, or `Distill`. The
[external backend example](testdata/public-api/main.go) uses an in-memory
store, compiles as a separate Go module, and runs the public workflows in CI.
`Workspace` is the interface for document reads, state reads, event reads and
writes, and conditional document updates. Each update includes the `Revision`
returned by the last state read. The revision lets bo detect a concurrent or
manual change; a backend should reject a stale revision.

`Distill` creates one `DistillationRecord` with exact raw and summary input
references and content digests. A backend implements this through
`CommitDistillation`; distillation documents use the `distillations/` document
kind and are create-only.

For a cloud caller, pass `SnapSourceConfig` in `SnapRequest` to disable local
Markdown reads and provide a controlled `http.Client`. The caller owns DNS,
redirect, and private-network policy for that client.

Public workflow errors use `bo.Error` and `bo.ErrorKind`. Use `bo.IsKind` or
`bo.IsAlreadyExists` instead of matching error strings.

## License

MIT
