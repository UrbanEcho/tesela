# Agent guide

Tesela is a semantic runtime library. It is not an agent: it has no model loop, prompts, planner, or provider code. Agent tooling consumes its capability catalog and lives outside this repository.

Read [`DESIGN.md`](DESIGN.md) for the API laws and [`CONTRIBUTING.md`](CONTRIBUTING.md) for the checks to run before reporting work as done. Do not restate either here.

## Where capability code lives

- `crates/tesela-capability`: catalog, invocation types, result handles, semantic diff. Lints are `deny(warnings)` and `deny(missing_docs)`.
- `crates/tesela-runtime/src/runtime/project.rs` builds catalogs, `schema.rs` builds JSON Schemas, `invoke.rs` dispatches invocations.
- `crates/tesela/src/raw.rs` re-exports engine vocabulary under `raw`; application-facing names stay out of it.
- `crates/tesela-py` is the PyO3 binding and `python/tesela` the facade. Native surface changes need matching updates to `python/tesela/_native.pyi`.

## Rules for capabilities

- `Runtime::invoke` must route through the same checked methods as typed plans (`search`, `get`, `mutate`, `traverse`, `execute_action`). Do not add a path that skips policy, authority, approvals, or revision checks.
- Exposure is token-based. Do not match capability names by substring.
- Catalog and invocation generation and fingerprint are checked before dispatch. Keep the stale-schema message actionable.
- Hidden capabilities must be rejected by `invoke`, not just omitted from catalogs.
- Retained result handles are execution-scoped and charged against the row budget. Serialized handles expose only `id` and `count`.
- Each `.rs`, `.py` and `.pyi` file under `crates/`, `python/` and `scripts/` stays under 500 lines; split a module before it reaches the limit.
- Record public behavior changes in `CHANGELOG.md` under `## Unreleased`.
