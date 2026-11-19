# Changelog

## Unreleased

### Breaking

- Removed `Query.offset`, `TraversalQuery.offset`, the facade `Query::offset` and `.offset()`, the `results.read` `offset` input and `ResultHandle::page(offset, ..)`. `Query` and `TraversalQuery` reject unknown keys, so a leftover `offset` from any client fails loudly.
- Pagination is keyset-only. The opaque `Cursor` (`Query::after(cursor)`, Python `.after(cursor)`) resumes strictly after a page's last row, and `Page.next_cursor` is typed (`Option<Cursor>`, Python `str | None`, and a `TypedDict` in `_native.pyi`) and absent exactly when no row follows. Python `Traverse` also takes `.after()`.
- Unsorted results are ordered by primary key, and the requested sort always ends in a primary-key ascending tiebreak. JSON `null` sorts as missing, before present values under `Asc`.
- `limit` must be in `1..=max_query_limit`: a value outside it is a validation error, never silently clamped. Python `Query.limit` rejects values below 1.
- Removed the 1,000-value handle reference cap (`MAX_REFERENCE_VALUES`): `$handle` and program `$ref` references expand every row, bounded only by the execution row budget; a `$handle` that retains just one page of a larger query (`next_cursor` present) is refused instead of expanding a partial key list. Programs and traversal follow cursors to completion (`ResultHandle::page(context, after, limit)`, `ResultHandle::iter`, `Traverse::after`), and the aggregate fallback walks cursors under a termination guard.
- `search` and `traverse` reject a sort on a property the caller's policy redacts, because the keyset cursor carries the last row's sort values. Traversal charges the row budget for returned rows only.
- An evicted handle reports that its request expired, and `RuntimeBuilder::max_retained_requests` is configurable.
- Native backends must return rows ordered by filter, sort and limit; the keyset position arrives folded into the filter, one extra row is requested, and `native_page` verifies the order and reports a contract error otherwise. A Python `Native.search` returns only records and never a cursor.
- All crates and the Python package are `0.3.0`.

### Fixed

- A cursor has no length bound, so a sort over long values (or long string keys) can always be followed to the last row. Program `$ref` expansion no longer fails when a record guard rejects a long run of backend pages. The `$handle` partial-handle error now points at program `$ref` and `results.read`. `python-test` rebuilds the extension first, and the `_native.pyi` stub imports `NotRequired` and `TypedDict` from `typing_extensions`.

### Added

- Added the opaque keyset `Cursor`, `KeysetPosition` and the shared record order (`Sort`, `SortDirection`, `compare_records`) to `tesela-ir`. Pagination has one mechanism: the requested sort plus a mandatory primary-key tiebreak, where missing and `null` sort together before present values under `Asc`. `tesela-backend` re-exports the order types unchanged.
- Added `conformance::verify_cursors` and `CursorFixture`, a backend conformance check driven by a seeding hook: cursor walks are complete, unique and in total order for every sort shape and page size, stable under ties and concurrent writes, end with no cursor on the last page, and reject foreign cursors, malformed cursors and non-positive limits. Memory, repository-scan and native adapters all run it.
- Replaced the Rust and Python application facades with immutable semantic definitions and typed plans executed only by `Runtime`.
- Added explicit contexts, optional lookup, typed execution metadata, suffix filtering, operator/type validation, native aggregate pushdown, streaming bounded repository scans, typed links and action commands, atomic batches, schema inspection, and curated raw namespaces.
- Removed public `Spec`, `BindingsBuilder`, wire mutations/queries, mutable Python query sets, class-level field descriptors, implicit local identity, string policy sentinels, and per-operation facade methods.
- Moved Python typing to inline source and added strict consumer fixtures.
- Capabilities now carry real JSON Schema `input_schema` values derived from object properties, and MCP-style `annotations` (`read_only`, `destructive`, `idempotent`, `open_world`).
- Capability exposure uses whole-token matching; substring matches no longer promote a capability.
- Added `Runtime::invoke` and `Runtime.invoke`, which route a `CapabilityInvocation` through the existing checked paths and return `InvocationOutput`. Retained results are returned as a bounded handle summary; the Python facade rejects `retain_results`.
- Stale-generation invocations now name both generations and tell the caller to fetch the catalog again.
- Breaking: removed `Capability.idempotent`; read idempotency from `Capability.annotations.idempotent`.
- Added a record-level policy hook: `PolicyEngine::guards_records` and `admit_record` let a host admit rows no `Filter` can express. Search refills pages the guard thins out, lookups and traversals refuse rejected records, and aggregation falls back to the residual engine so it only counts admitted rows.
- Added `fields` to `Query` and `TraversalQuery`: results keep only the listed properties (and the primary key). Unknown properties are rejected up front.
- Added a `{object}.aggregate` capability and `InvocationOutput::Aggregate`, and `ExecutionContext::remaining_budget`.
- Retained results now live in a bounded per-request store under stable ids (`h1`, `h2`, ...). Added `results.read` and `results.count_by` capabilities, `{"$handle": id, "field": name}` references in invocation input, `Runtime::handle` and `Runtime::release_handles`.
- Added `Runtime::run_program` with `Program` and `ProgramStep`: a straight-line program of capability calls with `{"$ref": step, "field": name}` references, validated whole (catalog identity, visible capabilities, references, revisions, approvals, backend-call budget) before its first step runs.
- Added `ModelReport`, a model-facing view of an error with its retryability and a recovery hint.
- Reworked engine internals for data-oriented execution with no public API change: allocation-free filters and grouping, one residual engine shared by every row layout, a per-type locked in-memory table with an undo-log batch and link index, lazy repository search encoding, and an aggregate fallback that walks keyset cursors to completion.

All package versions are `0.3.0`.
