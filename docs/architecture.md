# Architecture

## Dependency graph

```text
cmd/bo              -> bo API -> application -> domain
                              |              -> source
                              |              -> Storage contract
                              -> agent / provider contracts
storage adapters    -> application contracts and shared errors
source adapters     -> source contracts, domain, and shared errors
```

The root `bo` package exposes workflows and contracts. It does not select
concrete storage, source, or provider adapters.

`cmd/bo` is a consumer and composition root for local storage, workspaces, and
the DeepSeek provider. `application.Snap` owns the product-specific default
source assembly, so the CLI only opens storage and passes source inputs.

## Source workflow

The source package owns the adapter contracts:

1. ordered transports classify an input into a typed `Origin`;
2. the workflow looks up the plugin for that origin type;
3. the plugin returns a domain `RawSnapshot` with a transport-neutral
   `SourceKey`, title, and Markdown bytes.

The default workflow routes HTTP URLs to HTML or YouTube plugins and local
`.md` paths to the Markdown plugin. The URL and file adapters do not import
`application`. HTTP request policy remains inside the source plugins; storage
construction remains outside the use case.

## Internals

`internal/domain` owns stable product entities and the unchanged state format.
`internal/application` owns use-case orchestration and composes the default
source workflow. `internal/agent` owns the provider-neutral completion and tool
runtime. `internal/storage` owns filesystem and workspace adapters.

Categorized errors live in the dependency-neutral shared error package so
source, storage, and application code can use the same error vocabulary without
an adapter importing a use case.
