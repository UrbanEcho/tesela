//! Benchmarks for the in-memory backend's query path.
//!
//! The scenarios mirror the query shapes that dominate a real workload: an
//! unfiltered page, an equality filter, a `LIKE` scan, an `IN` set, a sorted
//! page, a deep offset and a grouped aggregation, each at two cardinalities so
//! the growth curve is visible rather than a single point.
//!
//! They are deliberately stated in terms of the query, not the implementation,
//! so they survive the move of this backend into its own crate and the
//! replacement of its private execution helpers with the shared runtime engine.
//! The numbers they produce before and after that move are the Phase 2
//! evidence.
//!
//! The backend is spec-free: it stores rows and answers queries, and the
//! primary key arrives already extracted by the caller.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::sync::Arc;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use tesela_backend::{
    AggregateQuery, Aggregation, AggregationFunction, Backend, LinkResolver, MemoryBackend,
    Mutation, Query, Sort, SortDirection,
};
use tesela_core::{ApiName, Error, ExecutionContext, FilterOp, Key, Principal, Value};
use tesela_ir::Filter;

/// The two cardinalities every query shape is measured at.
const SIZES: [usize; 2] = [10_000, 100_000];
const TENANTS: usize = 20;
const REGIONS: usize = 8;

/// Benchmark field names are compile-time constants, so validation is skipped.
fn name(value: &str) -> ApiName {
    ApiName::new_unchecked(value)
}

fn context() -> ExecutionContext {
    ExecutionContext::trusted(Principal::new("benchmark"), "benchmark").expect("context")
}

fn tenant(index: usize) -> String {
    format!("tenant_{:02}", index % TENANTS)
}

fn customer_key(index: usize) -> Key {
    Key::from(&Value::string(format!("customer_{index:06}")))
}

fn build_backend(rows: usize) -> Result<Arc<MemoryBackend>, Error> {
    let backend = MemoryBackend::new();
    let context = context();
    let object_type = name("customer");
    for index in 0..rows {
        let mut values = BTreeMap::new();
        values.insert(name("id"), Value::string(format!("customer_{index:06}")));
        values.insert(name("tenant_id"), Value::string(tenant(index)));
        values.insert(
            name("email"),
            Value::string(format!("user{index:06}@example.test")),
        );
        values.insert(name("revenue"), Value::float(index as f64 * 1.5));
        values.insert(
            name("region"),
            Value::string(format!("region_{:02}", index % REGIONS)),
        );
        backend.create(&context, &object_type, customer_key(index), values)?;
    }
    Ok(backend)
}

fn scalar(op: FilterOp, field: &str, value: Value) -> Filter {
    Filter {
        op,
        field: Some(name(field)),
        value: Some(value),
        values: Vec::new(),
        args: Vec::new(),
    }
}

fn page(limit: i32) -> Query {
    Query {
        limit: Some(limit),
        ..Query::default()
    }
}

/// The query shapes measured at every cardinality.
fn search_cases(rows: usize) -> Vec<(&'static str, Query)> {
    let in_values: Vec<Value> = (0..100)
        .map(|index| Value::string(format!("customer_{index:06}")))
        .collect();
    vec![
        // Unfiltered page: bounded execution stops after limit + 1 rows.
        ("no_filter_limit_100", page(100)),
        (
            "eq_filter_limit_100",
            Query {
                filter: Some(scalar(FilterOp::Eq, "tenant_id", Value::string(tenant(3)))),
                ..page(100)
            },
        ),
        // LIKE has to compile its pattern before it can match anything.
        (
            "like_filter_limit_100",
            Query {
                filter: Some(scalar(
                    FilterOp::Like,
                    "email",
                    Value::string("%0001%@example.test"),
                )),
                ..page(100)
            },
        ),
        // IN membership is tested per record.
        (
            "in_filter_limit_100",
            Query {
                filter: Some(Filter::is_in(name("id"), in_values)),
                ..page(100)
            },
        ),
        // Sorting on an unindexed property scans every match but retains only top-k.
        (
            "sorted_limit_100",
            Query {
                sort: vec![Sort {
                    property: name("revenue"),
                    direction: SortDirection::Desc,
                }],
                ..page(100)
            },
        ),
        // A deep offset: the engine walks past most of the table.
        (
            "deep_offset_limit_100",
            Query {
                offset: Some((rows - 1_000) as i32),
                ..page(100)
            },
        ),
    ]
}

fn bench_search(criterion: &mut Criterion) -> Result<(), Error> {
    let customer = name("customer");
    let context = context();
    let mut group = criterion.benchmark_group("memory_search");
    group.sample_size(30);

    for rows in SIZES {
        let backend = build_backend(rows)?;
        for (label, query) in search_cases(rows) {
            group.bench_with_input(BenchmarkId::new(label, rows), &query, |bencher, query| {
                bencher.iter(|| {
                    backend.search(black_box(&context), black_box(&customer), black_box(query))
                });
            });
        }
    }

    // A primary-key read is the one path that should not depend on the number
    // of stored rows, so it is measured across both cardinalities.
    let backend = build_backend(SIZES[0])?;
    let last = customer_key(SIZES[0] - 1);
    group.bench_function("get_by_primary_key", |bencher| {
        bencher.iter(|| backend.get(black_box(&context), black_box(&customer), black_box(&last)))
    });

    group.finish();
    Ok(())
}

fn aggregate_cases() -> Vec<(&'static str, AggregateQuery)> {
    vec![
        (
            "count_grouped_by_tenant",
            AggregateQuery {
                group_by: vec![name("tenant_id")],
                aggregations: vec![Aggregation {
                    function: AggregationFunction::Count,
                    property: None,
                    alias: "total".to_string(),
                }],
                ..AggregateQuery::default()
            },
        ),
        (
            "sum_and_max_grouped_by_region",
            AggregateQuery {
                group_by: vec![name("region")],
                aggregations: vec![
                    Aggregation {
                        function: AggregationFunction::Sum,
                        property: Some(name("revenue")),
                        alias: "revenue_sum".to_string(),
                    },
                    Aggregation {
                        function: AggregationFunction::Max,
                        property: Some(name("revenue")),
                        alias: "revenue_max".to_string(),
                    },
                ],
                ..AggregateQuery::default()
            },
        ),
    ]
}

fn bench_aggregate(criterion: &mut Criterion) -> Result<(), Error> {
    let customer = name("customer");
    let context = context();
    let mut group = criterion.benchmark_group("memory_aggregate");
    group.sample_size(30);

    for rows in SIZES {
        let backend = build_backend(rows)?;
        for (label, query) in aggregate_cases() {
            group.bench_with_input(BenchmarkId::new(label, rows), &query, |bencher, query| {
                bencher.iter(|| {
                    backend.aggregate(black_box(&context), black_box(&customer), black_box(query))
                });
            });
        }
    }

    group.finish();
    Ok(())
}

fn customer_values(index: usize, revenue: f64) -> BTreeMap<ApiName, Value> {
    BTreeMap::from([
        (name("id"), Value::string(format!("customer_{index:06}"))),
        (name("tenant_id"), Value::string(tenant(index))),
        (name("revenue"), Value::float(revenue)),
    ])
}

/// An atomic batch pays for its undo log, so it is measured against the rows
/// it rewrites rather than the table size.
fn bench_batch(criterion: &mut Criterion) -> Result<(), Error> {
    let customer = name("customer");
    let key = name("id");
    let context = context();
    let mut group = criterion.benchmark_group("memory_batch");
    group.sample_size(30);
    let backend = build_backend(SIZES[0])?;
    for width in [10_usize, 100, 1_000] {
        let mutations: Vec<Mutation> = (0..width)
            .map(|index| Mutation::Upsert {
                values: customer_values(index, index as f64),
                expected_revision: None,
                preconditions: Vec::new(),
            })
            .collect();
        group.bench_with_input(
            BenchmarkId::new("upsert_existing", width),
            &mutations,
            |bencher, mutations| {
                bencher.iter(|| {
                    backend.batch(
                        black_box(&context),
                        black_box(&customer),
                        black_box(&key),
                        black_box(mutations),
                    )
                });
            },
        );
    }
    group.finish();
    Ok(())
}

/// Link traversal reads a write-maintained index, so it should not scan the
/// target table once the index exists.
fn bench_traverse(criterion: &mut Criterion) -> Result<(), Error> {
    let order = name("order");
    let link = name("orders");
    let context = context();
    let mut group = criterion.benchmark_group("memory_traverse");
    group.sample_size(30);
    for rows in SIZES {
        let backend = MemoryBackend::new();
        backend.bind_link(link.clone(), name("customer_id"));
        for index in 0..rows {
            let values = BTreeMap::from([
                (name("id"), Value::integer(index as i64)),
                (
                    name("customer_id"),
                    Value::string(format!("c{}", index % 500)),
                ),
            ]);
            backend.create(&context, &order, Key::I64(index as i64), values)?;
        }
        let source = Key::string("c42");
        group.bench_function(BenchmarkId::new("linked_keys", rows), |bencher| {
            bencher.iter(|| {
                backend.linked_keys(
                    black_box(&context),
                    black_box(&link),
                    black_box(&order),
                    black_box(&source),
                )
            });
        });
    }
    group.finish();
    Ok(())
}

fn benches(criterion: &mut Criterion) {
    bench_search(criterion).expect("benchmark setup failed");
    bench_aggregate(criterion).expect("benchmark setup failed");
    bench_batch(criterion).expect("benchmark setup failed");
    bench_traverse(criterion).expect("benchmark setup failed");
}

criterion_group!(memory, benches);
criterion_main!(memory);
