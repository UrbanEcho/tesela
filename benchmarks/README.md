# Performance methodology

Criterion suites cover value/key behavior and the development backend query path. The public Rust example and Python script cover conversion plus runtime overhead.

```bash
cargo bench -p tesela-core --bench value
cargo bench -p tesela-backend --features dev --bench memory
cargo run --release -p tesela --example baseline
python benchmarks/baseline.py
```

Use a release build, record CPU/OS/compiler/interpreter versions, and compare the median of at least three same-host runs. `cargo bench --workspace --exclude tesela-py -- --test` is only a compile/panic smoke gate.

## Pre-refactor evidence

The 2026-09-19 Apple M5 Pro baseline identified the architectural hot spots:

| Operation, 10,000 rows | Median |
|---|---:|
| unsorted search → 100 | 1.11 ms |
| `LIKE` search → 100 | 1.73 ms |
| 100-value `IN` → 100 | 2.30 ms |
| grouped count | 1.83 ms |
| primary-key get | 206 ns |

These figures are evidence, not portable thresholds. Acceptance is also algorithmic: an unsorted limited scan stops after `limit + 1`; `IN` is linear in rows; `LIKE` compiles once; sorted bounded search retains top-k references rather than cloned matches; aggregation memory scales with groups; snapshot reads are lock-free.

## Profiling

Use Instruments on macOS for wall-time/allocation traces. DHAT or Callgrind can be enabled on Linux for allocation and instruction profiles. Profile only after a benchmark establishes a regression or uncertain hotspot. Rayon is intentionally absent until isolated sequential/parallel measurements demonstrate a stable crossover.

The active policy regression tests are in [`crates/tesela-runtime/tests/policy_isolation.rs`](../crates/tesela-runtime/tests/policy_isolation.rs). They verify filtered `get`, scoped mutation, source/target traversal isolation, redacted aggregation keys, and policy transformations as ordinary non-ignored tests.
