# Performance methodology

Criterion suites cover value/key behavior, the development backend query, batch and traversal paths, and the typed Rust facade over a repository scan. The public Rust example and Python script cover conversion plus runtime overhead.

```bash
cargo bench -p tesela-core --bench value
cargo bench -p tesela-backend --features dev --bench memory
cargo bench -p tesela --bench facade
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

## Data-oriented engine measurements

Measured on the same Apple M5 Pro with `cargo bench -p tesela-backend --features dev --bench memory -- --baseline before`, where `before` was recorded on the parent commit. Median change at 10,000 and 100,000 rows; the engine and public Rust surface are otherwise unchanged.

| Memory backend operation | 10,000 rows | 100,000 rows |
|---|---:|---:|
| unsorted search → 100 | +1.5% (noise) | +9.0% |
| equality filter → 100 | −53.1% | −43.5% |
| `LIKE` search → 100 | −6.5% | +6.6% |
| 100-value `IN` → 100 | −65.7% | −77.5% |
| sorted top-100 | −79.6% | −80.8% |
| deep offset → 100 | −34.3% | −20.4% |
| count by tenant | −89.3% | −94.5% |
| sum and max by region | −86.4% | −90.2% |
| primary-key get | −4.0% | |

The two unsorted/`LIKE` regressions are single-digit percentages on 10.7 µs and 18.7 µs medians, where the scan stops after 101 matches and the cost is fixed lookup overhead rather than throughput; the 10,000-row figures for the same queries are unchanged within noise or faster. New cases have no earlier baseline: an atomic batch of 10, 100 and 1,000 upserts costs 2.1 µs, 23.6 µs and 255 µs, and a link traversal returning 20 or 200 keys costs 90 ns or 382 ns at 10,000 or 100,000 rows, so it follows the result size rather than the table size.

The facade suite measures a closure repository whose scan clones its rows on every call, which dominates its repository figures: a 100-row page costs 59 µs at 1,000 rows and 393 µs at 10,000, equality 86 µs and 426 µs, top-100 138 µs and 704 µs, and a two-aggregate group-by 227 µs and 5.8 ms. The in-memory reference costs 15 µs, 21 µs, 56 µs and 22 µs at 1,000 rows. Read the repository figures as scan cost plus encoding of only the fields a query touches, not as a backend ceiling.

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
