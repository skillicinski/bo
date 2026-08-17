# Architecture

## Core rule

Business rules must not depend on command-line formatting, filesystems, HTTP,
or provider APIs. Dependencies point inward:

```text
presentation   ──> application ──> domain rules
infrastructure ──> application or domain-owned contracts
composition root ──> concrete pieces
```

The composition root may know every concrete type. Inner code should not choose
infrastructure implementations.

## Current structure

- `cmd/bo` dispatches commands and formats seed, snap, state, and agent output.
- The root `bo` package owns state types, use cases, validation, storage and
  provider contracts, and the bounded agent loop.
- `local` implements the filesystem storage contract.
- `source` implements HTTP, HTML extraction, and YouTube transcript fetching.
- `deepseek` implements the OpenAI-compatible completion contract.

The root package does not select concrete adapters. The command package wires
the local, source, and DeepSeek implementations together.

## Boundaries

Presentation parses transport input, invokes one use case, and formats its
result. Business decisions and concrete I/O do not belong in presentation.

Application code coordinates a user-visible workflow and returns explicit
success or failure. It may own a contract for an external capability when a
useful test or replaceable implementation requires one.

Domain rules express application meaning and invariants. They do not perform
I/O or read environment variables.

Infrastructure performs filesystem, HTTP, serialization, clock, and operating
system work. It contains integration mechanics, not business decisions.

## When to split code

Extract a boundary only when at least one of these is true:

- a use case needs testing without real I/O;
- a second implementation or entrypoint exists;
- unrelated capabilities force changes in the same module;
- a concrete dependency is leaking into business rules.

Keep related code together otherwise. Do not create layers, traits, shared
modules, or reusable libraries for hypothetical consumers.

## Go conventions

- Use structs and typed errors for meaningful domain values and failures.
- Use functions or small structs for workflows.
- Use interfaces only at real external boundaries.
- Prefer constructors, explicit parameters, and compile-time wiring.
- Keep provider and persistence types at their boundaries.
- Test behavior, not directory layout.

For each change, start from the user-visible use case, put each decision in its
owning boundary, and make the smallest structural change that keeps dependencies
pointing inward.
