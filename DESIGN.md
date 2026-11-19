# Tesela design

## API laws

1. Definitions describe meaning and never execute.
2. Plans are immutable, reusable values and never execute themselves.
3. `Runtime` is the sole application execution boundary.
4. Every execution has an explicit immutable `Context`; identity is never synthesized.
5. `run` returns the semantic value; `execute` adds receipt and revision metadata.
6. Policy, authority, cancellation, budgets, validation, and approvals are enforced before host effects.
7. Object values never carry hidden revisions or runtime state.
8. Backends and handlers are host bindings, not portable ontology data.
9. Atomic operations are used natively or rejected; they are never simulated with racy sequences.
10. Engine/interchange vocabulary stays in `raw`; application vocabulary stays small and typed.

## Runtime architecture

Runtime construction assembles an internal ontology and binding set from typed definitions. Compilation validates names, property types, links, action effects, inverses, and binding coverage; resolves semantic names to dense IDs and direct handles; and publishes an immutable snapshot. A replacement snapshot is compiled fully before one `ArcSwap` publication.

Execution validates context state and the plan before invoking policy or a backend. Policy is centralized and can add row filters, redactions, preconditions, transformations, approvals, and obligations. Receipts report the effective operation, backend, policy outcome, effects, fingerprint, generation, and timing.

## Data and performance invariants

- Records cross the engine boundary as typed scalar values and field maps; public objects are encoded and decoded once per boundary crossing.
- Pagination has exactly one mechanism: an opaque keyset cursor. The order is the requested sort followed by a mandatory primary-key tiebreak; unsorted queries are ordered by primary key. `limit` bounds only the page size, and following `next_cursor` until it is absent visits every matching row exactly once. A cursor is bound to its scope, filter and sort, so it cannot be replayed against a different query or row scope. A backend keeps at most `limit + 1` rows of page state: unsorted scans stop after one page and sorted scans retain bounded top-k state.
- The runtime follows cursors to completion wherever it consumes rows itself: `results.read`, traversal, program `$ref` reference expansion, and the aggregate fallback. None of them has a fixed total cap, and none truncates. A `$handle` reference expands every row of its handle, and a handle that retains only one page of a larger query (its `next_cursor` was present) is refused with an error that points at program `$ref` and `results.read` instead of expanding a partial key list. A page-size limit never bounds a `$ref`.
- A cursor token is a hex-encoded JSON position, not a secret. It carries the last row's sort values, so a sort on a property the caller's policy redacts is rejected (`policy_denied`) in `search` and `traverse`; otherwise `next_cursor` would leak the redacted value. Hex was chosen over base64 because `hex` is already a dependency and the tokens stay opaque; the length cost is accepted. Traversal charges the row budget only for the rows it returns, but each page re-reads the linked rows after the cursor, so its backend cost per page grows with the adjacency size.
- `ExecutionBudget.rows` is an explicit per-execution resource budget: exceeding it is an error, never a silent truncation. Each cursor page is a separate execution, so a walk is budgeted per page, not in total.
- Bounds on retained state are explicit errors, not evictions in disguise: `RuntimeBuilder::max_retained_requests` bounds how many requests keep result handles, an evicted handle reports that its request expired, and `MAX_HANDLES_PER_REQUEST` makes a request that retains too many handles fail with a validation error.
- Residual aggregation retains group state, not group rows. Groups are found by hashing borrowed values, so a row that joins an existing group allocates nothing.
- Filter fields resolve during compilation. Membership uses a compiled key set, `LIKE` uses a compiled pattern, and comparisons read stored values in place instead of projecting them to keys.
- One residual engine is generic over how rows expose fields. Borrowed records, owned records, dense backend rows, and lazily encoded repository objects all share it, so a repository scan encodes only the fields a query reads and only the rows it returns.
- The in-memory backend keeps one lock and one dense, dictionary-interned table per object type, so writes to different types never contend. Batches apply through an undo log instead of copying tables, and link traversal reads a write-maintained index.
- Byte-budget accounting measures payloads only when a byte limit is set.
- Native search and aggregate pushdown are selected only from advertised capabilities.
- Optional lookup is distinct from error-producing raw get.
- Immutable snapshots make concurrent reads independent of schema publication.

## Rust surface

`Object`, `Trait`, `Link`, and `Action` are semantic traits generated by derives. `Field<O, T>` binds expressions to an object and field type. The sealed `Plan` trait provides static output dispatch, so `Runtime::run(query::<Customer>(), &context)` returns `Page<Customer>` and an action returns its declared output type. Closure repositories cover small integrations; `backend::Native<T>` is the pushdown extension point.

## Python surface

`@object` is a frozen dataclass transform driven by annotations. It does not install query descriptors on model classes. Expressions start with `col` or `lit`, reject truth testing, and lower only at the native boundary. `Runtime` adapters translate repository callbacks to typed models and full contexts. Inline Python annotations are the public typing contract; only the compiled extension has a stub.

```python
@ts.object(key="id")
class Customer:
    id: str
    email: str | None = None

customers = ts.query(Customer).where(ts.col("email").ends_with("example.com"))
page = runtime.run(customers, context=context)
```

```rust
#[derive(Object)]
#[tesela(key = id)]
struct Customer { id: String, email: Option<String> }

let customers = query::<Customer>()
    .filter(Customer::email().ends_with("example.com"));
let page: Page<Customer> = runtime.run(customers, &context)?;
```

Links and action commands remain values as well: a link names source and target types while its resolver is bound by the runtime, and an action declares its effects and output while its handler is bound separately. Neither definition captures infrastructure or performs work.
