# Architecture

## Purpose and scope

This document defines the boundaries and design principles for the application.
It is a guide for keeping the system understandable, testable, and inexpensive
to change as it grows. It is not a required directory layout or a mandate to
build future capabilities in advance.

The principles apply to the current application and to each independently
deployable application that may be added later. Repository boundaries are an
operational decision, not an architectural layer.

Good structure reduces cognitive load. It does not require perfect
categorization, empty layers, or abstractions that have no current purpose.

## Core rule

Business rules must not depend on delivery mechanisms or external details.
Dependencies point inward:

```text
presentation   ──> application ──> domain
infrastructure ──> application and/or domain (implements inner-layer ports)
composition root ──> all concrete pieces (wiring only)
```

The composition root may know about every layer so it can construct the
application. Application and presentation code should not choose concrete
infrastructure implementations.

Dependency inversion means that an inner layer owns the contract it needs and
an outer layer implements it. A persistence boundary is one example: an
application may own a storage port, while a filesystem or database adapter
implements it. A separate repository layer or folder is optional.

## Responsibilities

### Domain

The domain expresses the meaning and constraints of the problem:

- entities and value objects
- invariants and domain operations
- domain errors and events, when they are needed

Domain code should not perform I/O, call external services, read environment
variables, or depend on a transport or persistence format. Domain operations
enforce business invariants; they do not orchestrate application workflows.

### Application

The application layer contains use cases and workflows. It coordinates
domain operations, applies application-level policies, and returns explicit
success or failure results.

It may define ports for external capabilities required by a use case. It must
not choose concrete databases, HTTP clients, provider APIs, filesystem paths,
or other infrastructure details.

Application code should be unit-testable without a real network, database, or
filesystem. Use a test implementation only when the use case has a real
boundary that needs one.

### Presentation

Presentation is how an external caller enters the system: a CLI, service API,
worker, or another adapter. It should remain thin and should:

1. receive input;
2. validate and normalize transport-level concerns;
3. invoke an application use case;
4. format the result and map errors to the protocol.

It must not contain business rules or call concrete infrastructure directly.
A small executable may combine presentation and composition-root responsibilities
in one entrypoint.

### Infrastructure

Infrastructure contains concrete integrations and side effects, such as:

- filesystem and database persistence
- HTTP and external-service clients
- clocks, randomness, and operating-system integration
- queues, email, logging, and provider-specific behavior
- serialization at an external boundary

Infrastructure implements ports required by inner layers. It may depend on
inner-layer types, but inner layers must not depend on its concrete types.
Infrastructure should contain integration mechanics, not business decisions.

## Organization over time

### Small application

When there is one dominant domain, simple horizontal organization is
appropriate. Global domain, application, and infrastructure modules—or a few
cohesive feature modules—are acceptable.

Keep related code together, keep the composition root obvious, and do not
create modules or traits solely to satisfy a diagram. Co-location is
acceptable while responsibilities and dependencies remain clear.

### Growing application

As the codebase grows, split cohesive capabilities within the existing
boundaries. Extract a port when it protects a real external boundary, enables a
useful test, or supports a genuinely replaceable implementation.

Prefer the smallest structural change that reduces cognitive load. Do not
extract a shared module merely because two pieces currently look similar;
shared concepts should be stable, small, and intentionally owned.

### Multiple domains or applications

When several domains emerge, organize by domain first and preserve the same
layering inside each domain. This prevents global layers from becoming
unrelated dumping grounds.

Domains must not import each other's internals. Pass explicit data across
domain boundaries, use an owned port where appropriate, or move a genuinely
shared and stable concept into a small shared area. Shared workflows do not
belong in shared code merely for convenience.

Each executable or service has its own composition root and presentation
adapters. Extract a reusable library or shared core only when there is a real
second consumer and a stable contract to share.

A distribution package, client library, or wrapper is an interface or
deployment boundary, not the source of business rules.

## Rust conventions

Clean Architecture is a dependency rule, not a required Rust directory
layout. Rust maps the ideas to:

- `struct`, `enum`, and `impl` for domain concepts;
- functions or small structs for use cases;
- traits only at real boundaries;
- constructors and explicit parameters for dependencies;
- `Result<T, E>` and meaningful error types for failure;
- a binary `main` or equivalent composition root for wiring.

Prefer direct construction and compile-time wiring. Use generics or
`dyn Trait` only when the resulting trade-off is useful. Keep transport,
provider, and persistence types at their boundaries instead of passing them
through the application core.

## Rules for development

For each feature:

1. Start with the user-visible use case.
2. Put business decisions in domain or application code.
3. Keep transport parsing and formatting in presentation.
4. Keep external effects and provider-specific behavior in infrastructure.
5. Wire concrete dependencies at the composition root.
6. Add an abstraction only when it protects a real boundary or enables a useful
   test.
7. Test behavior and dependency boundaries, not the presence of folders.

A refactor is justified when a module has unrelated reasons to change, a test
requires external services without needing to, a second entrypoint needs the
same use case, or a new feature must edit unrelated capabilities. Choose the
smallest move that addresses the pressure.

Prefer focused unit tests for domain and application behavior, plus integration
tests for presentation and external adapters.
