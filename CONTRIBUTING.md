# Contributing

Tesela is maintained as one Rust workspace plus its Python facade. Keep application APIs in `crates/tesela` and `python/tesela`; engine/interchange APIs belong in the lower crates and public `raw` namespaces.

Before submitting a change, run:

```text
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo doc --workspace --no-deps
uv run --group dev pytest python/tests -q
uv run --group dev pyright
python scripts/check_source_size.py
python scripts/check_release.py
```

Add behavior tests for public changes and focused lower-level tests for compiler, policy, concurrency, and bounded-execution invariants. Performance claims require same-host measurements with the command, profile, input size, and median of at least three runs recorded. Do not emulate atomic operations or weaken validation for a benchmark.

New public APIs must preserve the laws in [DESIGN.md](DESIGN.md). Compatibility aliases and transitional dual APIs are not accepted in the 0.2 facade.
