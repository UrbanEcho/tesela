# Tesela 0.2

Tesela is an embedded semantic capability runtime. A host supplies a semantic ontology, immutable bindings to its own repositories and services, an authorization policy, and an execution context. Tesela compiles those inputs into an immutable snapshot and executes queries, mutations, traversals, aggregations, and actions through one checked path.

Tesela is not a database, server, workflow engine, event bus, agent loop, or adapter collection. The ontology contains only traits, object types, link types, actions, and semantic constraints. Physical access and executable actions remain host code.

## Architecture

`Spec` is compiled into dense object/property/link/action IDs. During runtime construction every semantic name is resolved to a direct `Backend`, `LinkResolver`, or `ActionExecutor` handle. Readers load the active snapshot through `ArcSwap`; a replacement is compiled and bound completely before one atomic publication.

Every operation receives an `ExecutionContext`. Delegated authority, expiry, cancellation, deadlines, call/row/byte/external-cost budgets, then policy are checked in that order. Mutations and state-dependent actions support revisions and semantic preconditions. Successful executions return a machine-readable receipt. Observers are infallible notifications; durable audit belongs in the host transaction or outbox.

The runtime derives actor-specific `CapabilityCatalog`s from the compiled ontology, bindings, authority, policy, effects, and task relevance. Catalogs and calls carry the ontology fingerprint and generation, so stale calls fail explicitly.

## Rust

```rust
use std::collections::BTreeMap;
use std::sync::Arc;
use tesela::backend::BindingsBuilder;
use tesela::dev::MemoryBackend;
use tesela::model::{ApiName, DataType, ObjectType, Property, Spec, Value};
use tesela::policy::{AllowAllPolicy, ExecutionContext, Principal};
use tesela::query::Mutation;
use tesela::runtime::Runtime;

let customer = ApiName::from("customer");
let mut spec = Spec::default();
spec.upsert_object_type(ObjectType {
    api_name: customer.clone(),
    display: None,
    description: None,
    primary_key: ApiName::from("id"),
    properties: vec![Property {
        api_name: ApiName::from("id"),
        display: None,
        description: None,
        data_type: DataType::String,
        nullable: None,
        default: None,
        allowed_values: None,
        tags: Vec::new(),
    }],
    traits: Vec::new(),
    tags: Vec::new(),
    deprecated_at: None,
});

let backend = MemoryBackend::new();
let mut bindings = BindingsBuilder::new();
bindings.bind_object(customer.clone(), backend);
let runtime = Runtime::builder(spec, bindings.build(), Arc::new(AllowAllPolicy)).build()?;
let context = ExecutionContext::trusted(Principal::new("service"), "request-1")?;
runtime.mutate(&context, &customer, Mutation::Create {
    values: BTreeMap::from([(ApiName::from("id"), Value::string("c-1"))]),
})?;
# Ok::<(), tesela::error::Error>(())
```

Application code normally imports the curated `model`, `query`, `backend`, `policy`, `runtime`, and `error` modules. The optional `capability` feature exposes agent-facing catalogs and semantic diffs. `tesela::dev::MemoryBackend` exists for tests and local development.

Small repository integrations can use `BackendBuilder`; its advertised capabilities are derived from the handlers actually registered. Scan-oriented handlers can call `backend::execute_residual` to share Tesela's compiled filters, bounded top-k ordering, safe pagination, and early-stop behavior without reimplementing the query language. Atomic upsert and batch are available only when the host supplies native handlers.

## Python

```python
from dataclasses import dataclass
from tesela import MemoryBackend, Runtime, Spec

spec = Spec()

@spec.object_type(primary_key="id")
@dataclass
class Customer:
    id: str
    email: str | None

runtime = Runtime(spec, objects={Customer: MemoryBackend()}, policy="allow_all")
runtime.mutate(Customer, {"create": {"values": {"id": "c-1", "email": "a@example.test"}}})
customer = runtime.get(Customer, "c-1")
rows = runtime.objects(Customer).where(Customer.email == "a@example.test").take(10).load()
```

The PyO3 boundary converts dictionaries, lists, scalars, records, pages, queries, errors, and callbacks directly. Declared classes are the runtime interface. UUIDs, dates, datetimes, decimals, enums, nullable values, lists, and annotated vectors are supported. Action declaration classes are bound separately to real callables.

## Performance model

- semantic names resolve once to dense IDs and direct handles;
- snapshot reads are lock-free;
- `IN` uses hash membership and `LIKE` compiles once per query;
- unsorted limited scans stop after `limit + 1` matches and clone only returned rows;
- sorted bounded scans retain only the top `offset + limit + 1` record references;
- residual aggregation is one pass and retains state per group, not rows per group;
- upsert, conditional mutation, and batch are native backend primitives and are never synthesized.

Run `cargo bench -p tesela-core --bench value` and `cargo bench -p tesela-backend --features dev --bench memory`. See [benchmark methodology](benchmarks/README.md).

## Validation

`make verify` runs formatting, Clippy with all targets/features and warnings denied, workspace tests, benchmark smoke tests, packaging, Python tests/builds, documentation, and release checks. See [security boundaries](SECURITY.md) and the [Python bindings](python/README.md).

Licensed under Apache-2.0.
