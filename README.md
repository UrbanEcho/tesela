<div align="center">

# Tesela

**One typed, policy-checked interface over the data and services you already have.**

Define objects, relationships, and actions once. Bind them to the systems you already run. Query and execute them through the same semantic runtime from Rust, Python, or agent tooling.

[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](#license)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg?logo=rust)](https://www.rust-lang.org/)
[![Python](https://img.shields.io/badge/Python-3.10%2B-3776AB.svg?logo=python&logoColor=white)](https://www.python.org/)
[![Status](https://img.shields.io/badge/status-pre--1.0-yellow.svg)](#project-status)

</div>

---

Tesela is an embedded semantic runtime for applications that need to expose existing data and services through one consistent, policy-aware interface.

Your application defines the **meaning** of its domain:

- objects;
- properties;
- relationships;
- actions;
- constraints.

Your application continues to own the **implementation**:

- databases;
- APIs;
- repositories;
- services;
- transactions;
- infrastructure.

Tesela sits between the two.

It compiles the semantic model, binds it to the host's implementations, enforces policy and execution constraints, and exposes the resulting capabilities through the same checked execution path.

```mermaid
graph TD
    DM[Domain model]
    DM --> O[objects]
    DM --> L[links]
    DM --> A[actions]
    DM --> T[Tesela<br/>schema<br/>policy<br/>authority<br/>validation<br/>execution]
    T --> DB[database]
    T --> S[service]
    T --> API[API]
```

## Why Tesela?

Applications increasingly need the same domain operations in several places:

- ordinary application code;
- internal tools;
- automation;
- agent tools;
- administrative interfaces;
- policy-aware workflows.

Without a shared semantic layer, each surface tends to recreate its own query model, validation, authorization, action definitions, and integration logic.

Tesela lets the host define that contract once.

### One semantic model

Objects, links, actions, and constraints have one definition regardless of where their underlying data lives.

### Host-owned infrastructure

Tesela does not require moving application data into a new database or service.

Backends remain ordinary host code.

### Policy on every path

Queries, mutations, traversals, and actions pass through the same authorization and execution machinery.

### Application and agent surfaces from the same contract

Agent-facing capabilities can be projected from the same ontology and policy model used by ordinary application code.

### Rust engine, native Python interface

The execution engine is implemented in Rust. Python uses native PyO3 bindings over the same runtime rather than a separate client architecture.

## Project status

Tesela is currently **pre-1.0**.

The execution model and internal architecture are under active development, and public APIs may change while the Rust and Python contracts are finalized.

The repository currently builds releases from source. Registry installation instructions will be added when the first public release is published.

## Quick start

### Python

Define a domain object:

```python
from tesela import Object, Field, field

class Customer(Object):
    id: Field[str] = field(primary_key=True)
    email: Field[str | None] = field(default=None)
```

Bind it to an implementation and create a runtime:

```python
from tesela import AllowAll, MemoryBackend, Runtime

runtime = (
    Runtime.builder()
    .bind(Customer, MemoryBackend())
    .policy(AllowAll())
    .build()
)
```

Execution happens through a session representing one caller:

```python
session = runtime.session(principal="example-service")

session.create(
    Customer(
        id="c-1",
        email="a@example.com",
    )
)

customer = session.get(Customer, "c-1")

customers = (
    session.query(Customer)
    .where(Customer.email == "a@example.com")
    .limit(10)
    .all()
)
```

The model is semantic. `MemoryBackend` is only a local-development implementation; production applications bind the same `Customer` definition to their own repositories or services.

### Rust

```rust
use tesela::prelude::*;

#[derive(ObjectType)]
#[tesela(primary_key = "id")]
struct Customer {
    id: String,
    email: Option<String>,
}

fn main() -> Result<(), Error> {
    let runtime = Runtime::builder()
        .bind::<Customer>(MemoryBackend::new())
        .policy(AllowAll)
        .build()?;

    let session = runtime.session(
        Context::service("example-service")?
    );

    session.create(Customer {
        id: "c-1".into(),
        email: Some("a@example.com".into()),
    })?;

    let customer = session.get::<Customer>("c-1")?;

    let customers = session
        .query::<Customer>()
        .filter(Customer::email().eq("a@example.com"))
        .limit(10)
        .all()?;

    Ok(())
}
```

## Mental model

Tesela separates four concerns that are often mixed together.

| Concern | Tesela concept | Example |
| --- | --- | --- |
| What exists? | Object types, links, actions | `Customer`, `CustomerOrders` |
| Where does it come from? | Backend bindings | Postgres repository, HTTP service |
| Who may use it? | Policy + authority | tenant, role, delegated scope |
| Who is calling now? | Session / execution context | request, service, agent |

This separation is fundamental.

An object declaration never contains a database table name.

A relationship declaration never requires a foreign-key implementation.

An action declaration describes semantics and effects; the host binds the executable implementation separately.

## Objects

Objects define semantic entities.

```rust
#[derive(ObjectType)]
#[tesela(primary_key = "id")]
struct Customer {
    id: String,
    email: Option<String>,
    active: bool,
}
```

The declaration describes what a customer means to Tesela.

It does not describe where customers are stored.

The same object may therefore be backed by a database repository, remote service, in-memory implementation, or application-specific adapter.

## Queries

Queries are compiled against the semantic model before reaching a backend.

```rust
let customers = session
    .query::<Customer>()
    .filter(
        Customer::active()
            .eq(true)
            .and(Customer::email().is_not_null())
    )
    .limit(100)
    .all()?;
```

Unknown fields and invalid operations fail before execution rather than silently reaching a backend.

Backends may execute supported operations natively. Tesela can also provide shared residual execution for integrations that expose simpler scan primitives.

## Links

Links describe relationships independently from their physical implementation.

Conceptually:

```text
Customer ── placed ──▶ Order
```

The relationship may be implemented by:

- a foreign key;
- a join table;
- a graph lookup;
- another service;
- arbitrary application code.

Tesela sees the semantic relationship.

The host owns the resolver.

## Actions

Actions describe executable domain operations.

An action can carry information such as:

- its subject;
- input schema;
- read/write/external effects;
- risk;
- idempotency;
- concurrency expectations.

The declaration and implementation remain separate.

That means the same semantic action can participate in policy decisions and capability discovery without embedding application infrastructure into the ontology.

## Runtime and sessions

A `Runtime` represents compiled application semantics:

```text
Runtime
├── ontology
├── backend bindings
├── action bindings
├── link resolvers
└── policy
```

A `Session` represents one execution authority:

```text
Session
├── principal
├── delegated authority
├── request identity
├── deadline
├── cancellation
└── execution budget
```

This makes request-specific state explicit without requiring applications to rebuild the semantic runtime for every call.

## Policy and authority

Policy is evaluated by the runtime and enforced by the runtime.

A policy decision may:

- allow an operation;
- deny it;
- restrict visible rows;
- redact properties;
- constrain execution.

Tesela applies these restrictions through the same checked path used by reads, mutations, traversals, and actions.

Execution contexts can additionally carry delegated authority, expiry, cancellation, deadlines, and resource budgets.

## Capabilities for agents

Tesela can derive actor-specific capabilities from:

```text
ontology
+ bindings
+ authority
+ policy
+ action effects
+ task relevance
```

The resulting catalog describes what a caller can actually do rather than merely listing every operation that exists.

Capability catalogs and invocations carry the ontology fingerprint and generation so calls against stale semantic state fail explicitly.

This allows ordinary application code and agent tooling to share the same underlying authority and execution model.

## Existing infrastructure stays yours

Tesela is not a database.

It does not own your connection pool, transaction manager, HTTP client, message bus, application services, or deployment model.

A backend implements the operations its underlying system can perform.

```text
Tesela semantic operation
          │
          ▼
   host backend binding
          │
    ┌─────┼─────┐
    ▼     ▼     ▼
 Postgres API  service
```

Unsupported operations fail explicitly rather than being silently synthesized from weaker primitives where doing so would change semantics.

## Compiled execution model

At runtime construction, Tesela compiles semantic names into dense internal identifiers and resolves them to their corresponding backend, link, and action handles.

The resulting runtime snapshot is immutable.

Readers access the active snapshot without taking a global runtime lock, while replacement ontologies are fully compiled and bound before publication.

A failed replacement therefore leaves the previous valid runtime in service.

## Execution receipts

Checked executions can produce machine-readable receipts describing what happened.

Receipts make execution observable without turning observation callbacks into the durability mechanism.

Applications requiring durable audit trails should commit them through the same transaction or outbox discipline used for the operation being audited.

## Performance model

Tesela is designed so semantic abstraction does not require repeatedly rediscovering runtime structure.

Important properties include:

- semantic names resolve during compilation;
- runtime snapshots are immutable;
- active snapshot reads avoid a global lock;
- filters are compiled before backend execution;
- bounded scans can terminate early;
- bounded sorting retains only the required candidate set;
- residual aggregation retains aggregate state rather than full groups;
- backend-native atomic operations remain native.

Benchmark methodology and sources are documented under [`benchmarks/`](benchmarks/).

Performance claims should be interpreted together with the workload, backend, hardware, and Tesela revision used to produce them.

## Rust architecture

The workspace is divided by responsibility:

```text
tesela-core        common values, identity, errors, execution primitives
tesela-ir          semantic declarations and query representation
tesela-backend     backend and policy contracts
tesela-runtime     compilation and checked execution
tesela-capability  capability projection
tesela-macros      declarative Rust macros
tesela             curated Rust API
tesela-py          native Python boundary
```

Applications should normally depend on the `tesela` facade rather than individual engine crates.

## Python architecture

The Python package is a native facade over the Rust runtime.

```text
Python models and expressions
          │
          ▼
      tesela._native
          │
          ▼
    Rust runtime engine
```

Python owns declaration ergonomics, model materialization, expression syntax, and Python callback adaptation.

Compilation, policy enforcement, authority checks, backend dispatch, and execution remain in the Rust engine.

## Development

Clone the repository and verify the complete workspace:

```bash
make verify
```

The verification pipeline includes:

- Rust formatting;
- Clippy with warnings denied;
- workspace tests;
- documentation builds;
- benchmark smoke tests;
- Python tests;
- Pyright;
- Python package builds;
- wheel builds;
- release checks.

Individual Rust tests can be run with:

```bash
cargo test --workspace
```

Python bindings can be developed locally with Maturin.

## Security

Tesela sits on an authorization and execution boundary, so security behavior is considered part of the runtime contract.

See [`SECURITY.md`](SECURITY.md) for the documented trust boundaries, guarantees, and reporting process.

## License

Tesela is released under the [Apache License 2.0](LICENSE).
