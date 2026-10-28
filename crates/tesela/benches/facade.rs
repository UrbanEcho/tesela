//! Benchmarks for the typed facade over a host-owned repository.
//!
//! A repository scan hands the facade typed values. These cases measure what
//! the facade adds on top of the scan itself: encoding only the fields a query
//! reads, pruning rejected rows before they are encoded, and streaming
//! aggregation, each against the in-memory backend as the reference.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use tesela::prelude::*;
use tesela::{count, create, query};

#[derive(Debug, Clone, PartialEq, Object)]
#[tesela(key = id)]
struct Customer {
    id: String,
    region: Option<String>,
    email: String,
    spend: i64,
}

const SIZES: [usize; 2] = [1_000, 10_000];

fn customers(rows: usize) -> Vec<Customer> {
    (0..rows)
        .map(|index| Customer {
            id: format!("customer_{index:06}"),
            region: Some(format!("region_{}", index % 8)),
            email: format!("user{index:06}@example.test"),
            spend: i64::try_from(index % 997).expect("small"),
        })
        .collect()
}

fn context() -> Context {
    Context::builder("benchmark")
        .request_id("benchmark")
        .build()
        .expect("context")
}

fn repository(rows: &[Customer]) -> Runtime {
    let rows = rows.to_vec();
    Runtime::builder()
        .object::<Customer>(
            backend::<Customer>()
                .scan(move |_| Ok(rows.clone()))
                .build(),
        )
        .policy(tesela::policy::allow_all)
        .build()
        .expect("runtime")
}

fn memory(rows: &[Customer], context: &Context) -> Runtime {
    let runtime = Runtime::builder()
        .object::<Customer>(MemoryBackend::new())
        .policy(tesela::policy::allow_all)
        .build()
        .expect("runtime");
    for row in rows {
        runtime.run(create(row.clone()), context).expect("create");
    }
    runtime
}

fn bench_search(criterion: &mut Criterion) {
    let context = context();
    let mut group = criterion.benchmark_group("facade_search");
    group.sample_size(30);
    for size in SIZES {
        let rows = customers(size);
        let runtimes = [
            ("repository", repository(&rows)),
            ("memory", memory(&rows, &context)),
        ];
        for (name, runtime) in &runtimes {
            group.bench_function(
                BenchmarkId::new(format!("{name}_page_100"), size),
                |bench| {
                    bench.iter(|| runtime.run(black_box(query::<Customer>().limit(100)), &context));
                },
            );
            group.bench_function(BenchmarkId::new(format!("{name}_eq_100"), size), |bench| {
                bench.iter(|| {
                    runtime.run(
                        black_box(
                            query::<Customer>()
                                .filter(Customer::region().eq("region_3"))
                                .limit(100),
                        ),
                        &context,
                    )
                });
            });
            group.bench_function(BenchmarkId::new(format!("{name}_top_100"), size), |bench| {
                bench.iter(|| {
                    runtime.run(
                        black_box(
                            query::<Customer>()
                                .order_by(Customer::spend().desc())
                                .limit(100),
                        ),
                        &context,
                    )
                });
            });
        }
    }
    group.finish();
}

fn bench_aggregate(criterion: &mut Criterion) {
    let context = context();
    let mut group = criterion.benchmark_group("facade_aggregate");
    group.sample_size(30);
    for size in SIZES {
        let rows = customers(size);
        let runtimes = [
            ("repository", repository(&rows)),
            ("memory", memory(&rows, &context)),
        ];
        for (name, runtime) in &runtimes {
            group.bench_function(
                BenchmarkId::new(format!("{name}_by_region"), size),
                |bench| {
                    bench.iter(|| {
                        runtime.run(
                            black_box(
                                query::<Customer>()
                                    .group_by(Customer::region())
                                    .agg([count("n"), Customer::spend().sum("total")]),
                            ),
                            &context,
                        )
                    });
                },
            );
        }
    }
    group.finish();
}

criterion_group!(facade, bench_search, bench_aggregate);
criterion_main!(facade);
