# Changelog

## Unreleased

- Replaced the Rust and Python application facades with immutable semantic definitions and typed plans executed only by `Runtime`.
- Added explicit contexts, optional lookup, typed execution metadata, suffix filtering, operator/type validation, native aggregate pushdown, streaming bounded repository scans, typed links and action commands, atomic batches, schema inspection, and curated raw namespaces.
- Removed public `Spec`, `BindingsBuilder`, wire mutations/queries, mutable Python query sets, class-level field descriptors, implicit local identity, string policy sentinels, and per-operation facade methods.
- Moved Python typing to inline source and added strict consumer fixtures.

All package versions remain `0.2.0`.
