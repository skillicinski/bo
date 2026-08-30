# Architecture

## Dependency graph

```text
external Go consumer       -> bo
cmd/bo                     -> bo
evals/cmd/bo-eval          -> application, provider/deepseek, storage/local
bo                         -> application, agent, domain, errors, provider/deepseek, provider/gemini, source, source/file, source/url, storage/local
application                -> agent, domain, errors, source
source                     -> domain, errors
source/file                -> source, domain, errors
source/url                 -> source, domain, errors
storage/local              -> application, domain, errors
provider/deepseek          -> agent, errors
provider/gemini            -> agent, errors, oauth2/google
agent                      -> errors
domain                     -> errors
```

Go's `internal/` rule prevents an external module from importing implementation
packages. The root `bo` package is the supported external Go surface. The
separate consumer module in `testdata/public-api` compiles that boundary in CI.

## Terms

- **Port:** a Go interface at a boundary. `bo.Workspace` is a port for storage;
  a workflow can use a local, memory, or remote implementation.
- **Composition root:** the package that selects concrete implementations and
  connects them. `cmd/bo` selects the local workspace and calls `bo`.
- **Aggregate:** one state object that keeps related records and their rules
  together. A `SourceRecord` groups one source, its snapshots, and its summary.
- **Opaque revision:** a value that callers may compare and pass back, but not
  inspect. It lets a workspace reject an update based on stale state.

## Layer contracts

### `bo` public package

The public package exposes workflows and stable data types.

- Owns: requests, results, public errors, state types, workspace ports, source
  composition, and supported constructors.
- Does not own: CLI parsing, source routing, storage format, or provider HTTP.
- Calls: application workflows and supported internal adapters.

### `cmd/bo`

The CLI converts arguments and results into a process interface.

- Owns: argument parsing, stdout and stderr, exit codes, and local dependency
  selection.
- Does not own: workflow rules, source fetching, or workspace persistence.
- Calls: only the public `bo` package.

### `internal/application`

The application layer runs the workflows and records operation events.

- Owns: workflow orchestration, synthesis stage selection, validation order,
  and operation outcomes.
- Does not own: CLI output, workspace selection, or local file operations.
- Calls: domain types, the workspace port, the agent runtime, and the `source`
  fetch port.

### `internal/domain`

The domain layer defines the state and operation rules.

- Owns: source records, snapshots, summaries, operations, and validation.
- Does not own: filesystem, network, process, or provider behavior.
- Calls: shared error definitions for validation failures.

### `internal/source`

The source layer routes an input to a source adapter.

- Owns: transport and plugin interfaces, input classification, and source
  identity rules.
- Does not own: application workflows, workspace writes, or CLI output.
- Calls: domain types and shared errors.

`source/file` reads local Markdown. `source/url` fetches HTML and YouTube
transcripts. These adapters return the same domain snapshot shape.

### `internal/source/file` and `internal/source/url`

The source adapters translate external or local input into domain snapshots.

- Owns: file reads, HTTP requests, HTML conversion, and YouTube transcript
  handling.
- Does not own: source routing, workflow orchestration, or workspace writes.
- Calls: `source`, domain types, shared errors, and their transport libraries.

### `internal/agent`

The agent layer runs bounded provider-neutral completion loops.

- Owns: turns, tool calls, limits, and provider-neutral messages.
- Does not own: provider-specific HTTP, workspace selection, or CLI rendering.
- Calls: the shared error package and the provider completion contract.

### `internal/provider/deepseek`

The DeepSeek adapter translates the provider HTTP protocol.

- Owns: DeepSeek requests, responses, and transport errors.
- Does not own: the agent loop, workspace selection, or CLI rendering.
- Calls: the agent contract and shared errors.

### `internal/provider/gemini`

The Gemini adapter translates the native `generateContent` protocol for the
Gemini Developer API and Vertex AI.

- Owns: Gemini requests, responses, API-key headers, Vertex AI endpoint paths,
  and ADC access tokens.
- Does not own: the agent loop, workspace selection, or CLI rendering.
- Calls: the agent contract, shared errors, and Google's ADC OAuth2 package.

### `internal/storage/local`

The local adapter stores one named workspace on disk.

- Owns: workspace selection, Markdown files, `state.json`, `log.jsonl`, and
  recovery-safe writes.
- Does not own: workflow orchestration, source fetching, or CLI output.
- Calls: application contracts, domain types, and shared errors.

### `internal/errors`

The shared error package gives internal layers one failure vocabulary.

- Owns: error kinds, retryability, and context classification.
- Does not own: workflow policy, output formatting, or HTTP status selection.
- Calls: standard-library error behavior only.

### `evals/cmd/bo-eval`

The evaluation command is a separate composition root for controlled tests.

- Owns: evaluation setup and explicit tool selection.
- Does not own: production CLI behavior or the public API contract.
- Calls: selected internal application, provider, and storage packages.

## Workspace contract

A workspace stores the documents, inventory, and event log for one named
collection.

It provides:

- document reads;
- inventory reads;
- event reads and writes;
- conditional snapshot, summary, and distillation-document updates.

Each update includes a revision. The revision lets bo detect whether another
process or a manual edit changed the workspace. A local workspace advances the
revision after each successful content update and rejects stale updates.

Workflows receive an already-open workspace. They do not select its location or
close it. The caller owns workspace lifetime.

## Snap flow

For `bo snap notes ./note.md`, the flow is:

1. `cmd/bo` opens `notes` with the local manager.
2. `cmd/bo` calls `bo.Snap` with the workspace and source input.
3. The public package composes the default URL and local Markdown adapters and
   converts the request to the application contract.
4. The application reads the current state and revision.
5. The source workflow tries URL routing, then local Markdown routing.
6. The selected adapter returns a title, source key, and bounded Markdown bytes.
7. The application validates the result and chooses a document filename.
8. The workspace commits the document, state, and operation event with the
   expected revision.
9. The application returns a `SnapResult`; the CLI renders the outcome.

Cloud callers may disable local Markdown sources and provide an `http.Client`.
The caller owns that client's DNS, redirect, and private-network policy. The
source adapters apply one bounded read limit to local Markdown and all URL
responses before parsing them.

An external Go caller starts at step 2 and can provide any `bo.Workspace`
implementation.

## State model

`State` contains one `SourceRecord` for each source. A source record contains
the source key, immutable raw snapshots, and at most one current summary. The
summary must refer to a snapshot in the same record. This is the aggregate rule
that keeps related state consistent.

State also contains distillation records. A distill record stores its kind,
topic, timestamps, content baseline, and every raw or current-summary input
with its source identity and content digest. It must reference documents from
at least two source identities. A matching topic may update an existing record
in place while preserving its creation time.

## Synth flow

For `bo synth notes`, the application first summarizes raw documents without a
current summary, then reads the newest raw snapshot for each source and
includes a summary only when it derives from that snapshot. `summarize` and
`distill` select one stage. The distill stage processes all useful unprocessed
topics in one bounded agent runtime and ends with an explicit skip result.

The distill agent may read current raw and summary documents, inspect existing
distillation candidates for topic matching, and call `skip_distill`,
`write_distillation`, or `edit_distillation`. The host validates every evidence
reference, computes its digest, renders deterministic Markdown, and commits the
document, state, and mutation event in one conditional transaction. Matching
requires the topic and the complete set of input references and digests. An
unchanged topic and input set is skipped. The public `Synth` result reports
committed documents grouped by mutation operation.

## Optional reading

- [Aggregation and composition](https://atomicobject.com/oo-programming/object-oriented-aggregation)
- [DDD aggregates](https://martinfowler.com/bliki/DDD_Aggregate.html)
