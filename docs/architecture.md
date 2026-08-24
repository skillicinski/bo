# Architecture

## Dependency graph

```text
cmd/bo                    -> bo API -> application -> domain
                                              |     -> source
                                              |     -> public domain and workspace contracts
                                              -> agent / provider contracts
evals/cmd/bo-eval         -> application -> agent / provider adapters
supported adapters        -> bo contracts and internal adapters
source adapters           -> source contracts, domain, and shared errors
```

The root `bo` package owns the workflow requests, results, domain contracts,
and supported local and DeepSeek constructors. It does not expose source
routing or the generic agent protocol.

`cmd/bo` is the production presentation layer and uses only the public `bo`
API. `evals/cmd/bo-eval` is an evaluation-only composition root for explicit
tool-set selection. `application.Snap` owns the product-specific default source
assembly, so the production CLI only opens a scoped workspace and passes source
inputs.

## Internals

`internal/domain` owns stable product entities and the source-aggregate state format.
`internal/application` owns use-case orchestration and composes the default
source workflow. `internal/agent` owns the provider-neutral completion and tool
runtime. `internal/storage` owns workspace adapters; application workflows use
the workspace persistence port and do not receive filesystem paths.

Stable error kinds live in the dependency-neutral shared error package so
source, storage, and application code can use the same error vocabulary without
an adapter importing a use case. The root package translates those kinds into
one public error contract and keeps HTTP protocol status selection outside bo.

## Source workflow

The source package owns the adapter contracts:

1. ordered transports classify an input into a typed `Origin`;
2. the workflow looks up the plugin for that origin type;
3. the plugin returns a domain `RawSnapshot` with a transport-neutral
   `SourceKey`, title, and Markdown bytes.

The default workflow routes HTTP URLs to HTML or YouTube plugins and local
`.md` paths to the Markdown plugin. The URL and file adapters do not import
`application`. HTTP request policy remains inside the source plugins; storage
construction remains outside the use case. Exact HTTP and HTTPS URLs with
non-credential query parameters are valid source identities; URL user
information and credential-bearing query parameters, including signed AWS and
GCS URLs, are rejected.

### SourceRecord

The `SourceRecord` object contains related parts, snapshots and one summary, so it has that whole/part shape. Atomic Object
describes this containment relationship (https://atomicobject.com/oo-programming/object-oriented-aggregation).

Our generalized term for this type of records is "aggregate", which takes it's meaning from Domain-Driven Development (DDD):

```
  State
  └── SourceRecord             aggregate root: SourceKey
      ├── Snapshots[]          immutable child records
      └── Summary              optional current child record
```

SourceRecord groups data that must remain consistent:

- one exact `SourceKey`;
- many snapshots;
- at most one summary;
- the summary must reference one snapshot in the same source.

That matches DDD’s aggregate concept: a cluster with a root that protects its invariants.
Fowler’s definition (https://martinfowler.com/bliki/DDD_Aggregate.html) also treats
aggregates as consistency and persistence boundaries.

## Workspace abstraction

A workspace is the persistence boundary for one scoped workspace. The port
lists and reads documents, loads state with an opaque revision, and accepts a
semantic snapshot or summary commit only with that revision. The local
revision covers serialized state and the contents of all raw and summary
documents, so external edits produce a typed conflict and bo does not
reconcile them.

The workspace has separate durable artifacts: `state.json` is the document
inventory, and `log.jsonl` is the append-only event ledger. Event appends and
bounded event reads belong to the workspace port. The ledger is outside the
content revision, so recording a read does not invalidate an unrelated content
mutation. Local mutation journals include the document, inventory, and event
append in one recovery boundary.

Adapters own publication, cleanup, rollback, and atomicity. The local adapter
stages document and state writes, syncs each destination parent directory, and
restores the previous state and document content when a commit fails. A durable
transaction marker records a rollback-safe prepared phase, records commit only
after both new artifacts are durable, and recovers a process crash before reads
resume. Every adapter must advance its revision for each successful mutation.
Workspace selection and lifetime ownership remain at the API boundary;
application workflows use an already-open workspace and do not close it.
