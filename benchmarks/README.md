# Performance methodology

Criterion suites cover value/key behavior and the development backend query path. The public Rust example and Python script cover conversion plus runtime overhead.

```bash
cargo bench -p tesela-core --bench value
cargo bench -p tesela-backend --features dev --bench memory
uv run python benchmarks/baseline.py
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

## Public-facade integration measurements

Measured on 2026-09-21 on an Apple M5 Pro (`arm64`), Rust 1.98.1 and Python 3.14.7. Each old-facade figure is the median of three complete 10,000-operation runs from commit `bae546b`; each new-facade figure is the median of three loops in the checked-in benchmark.

| Public Python operation | Old facade | Immutable-plan facade |
|---|---:|---:|
| query and materialize 100 typed rows | 129.20 µs | 154.11 µs |
| typed primary-key get | 3.78 µs | 11.55 µs |
| typed upsert | 3.57 µs | 13.61 µs |

These are intentional facade-cost regressions, not engine throughput improvements. The new calls construct or dispatch a typed plan, require and lower a full immutable context, return semantic typed values, and create execution metadata; the old mutation benchmark returned the raw execution dictionary. The comparable 100-row query adds about 24.9 µs (19.3%), primarily fixed Python dispatch/context/result construction around 100 native materializations. Primary-key get and upsert expose that fixed cost most clearly. Engine data structures and residual algorithms remain covered by the Criterion suites below; no speedup is claimed from this facade refactor.

New-facade medians across the same three complete runs were 2.12 µs for query-plan construction, 49.36 µs for a filtered bounded 25-row query, and 16.17 µs for a count aggregation over 100 rows. These results establish an explicit baseline for subsequent lowering and callback-crossing work.

## Profiling

Use Instruments on macOS for wall-time/allocation traces. DHAT or Callgrind can be enabled on Linux for allocation and instruction profiles. Profile only after a benchmark establishes a regression or uncertain hotspot. Rayon is intentionally absent until isolated sequential/parallel measurements demonstrate a stable crossover.

The active policy regression tests are in [`crates/tesela-runtime/tests/policy_isolation.rs`](../crates/tesela-runtime/tests/policy_isolation.rs). They verify filtered `get`, scoped mutation, source/target traversal isolation, redacted aggregation keys, and policy transformations as ordinary non-ignored tests.
