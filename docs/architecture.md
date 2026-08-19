# Architecture

## Layout

```text
cmd/bo/                 CLI parsing, output, and dependency wiring
api.go                  public Go façade for workflows and contracts
internal/agent/         provider-neutral agent runtime and tool boundary
internal/application/   use-case orchestration
internal/domain/        private state and document entities
internal/provider/      provider adapters
internal/source/url/     URL, HTML, and YouTube source adapter
internal/storage/        filesystem and workspace adapters
```

The root `bo` package exposes the reusable workflows and contracts through
`api.go`. It does not select concrete providers, sources, or storage
implementations.

`cmd/bo` is the composition root. It selects local storage and workspace
adapters, the HTTP source, and the DeepSeek provider. It parses CLI input and
formats CLI output.

`internal/domain` owns the state and document entities used by the workflows and
storage adapter. It has no dependency on the root package or external adapters.

`internal/agent` owns the provider-neutral agent runtime. A use case supplies a
provider and the tool set it permits.

`internal/application` owns the use-case workflows, including `seed`, `snap`,
`state`, and `synth`. It depends on domain types and application contracts, not
on concrete adapters. Synthesis supplies its own bounded local tools to the
agent runtime.

`internal/application/contracts.go` owns the storage, source, and workspace
contracts used by application code and adapters. `internal/agent` owns the
completion and tool contracts. The root API re-exports both contract groups
for callers that need to compose the package.

Storage and source adapters implement the application contracts; provider
adapters implement the agent completion contract. The root package does not
import the adapters.

### Domain

Representations of stable product concepts, independent rules and lifecycles. They are long-lived and can be abstracted into use cases.

### Application

Abstractions that orchestrate the internal functionality and domain entities of the system.
