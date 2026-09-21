//! Microbenchmarks for `Value` ordering/hashing and API-name validation.
//!
//! These target the paths beneath sorting, grouping, backend keying, and
//! validated semantic-name construction. Comparisons recurse through borrowed
//! array and object elements without intermediate allocations.

use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};
use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use tesela_core::{ApiName, Value};

fn object_value(keys: usize, seed: u64) -> Value {
    let mut map = serde_json::Map::with_capacity(keys);
    for index in 0..keys {
        map.insert(
            format!("field_{index:02}"),
            serde_json::Value::from(seed.wrapping_mul(index as u64 + 1)),
        );
    }
    Value::new(serde_json::Value::Object(map))
}

fn array_value(items: usize, seed: u64) -> Value {
    let values = (0..items)
        .map(|index| serde_json::Value::from(seed as i64 + index as i64))
        .collect();
    Value::new(serde_json::Value::Array(values))
}

fn bench_ordering(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("value_ord");

    let left_string = Value::string("customer_000042");
    let right_string = Value::string("customer_000043");
    group.bench_function("scalars", |bencher| {
        bencher.iter(|| black_box(&left_string).cmp(black_box(&right_string)))
    });

    let left_object = object_value(8, 7);
    let right_object = object_value(8, 11);
    group.bench_function("objects_8_keys", |bencher| {
        bencher.iter(|| black_box(&left_object).cmp(black_box(&right_object)))
    });

    let left_array = array_value(16, 3);
    let right_array = array_value(16, 5);
    group.bench_function("arrays_16_items", |bencher| {
        bencher.iter(|| black_box(&left_array).cmp(black_box(&right_array)))
    });

    group.finish();
}

fn bench_hashing(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("value_hash");

    let value = Value::string("customer_000042");
    group.bench_function("scalar", |bencher| {
        bencher.iter(|| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            black_box(&value).hash(&mut hasher);
            hasher.finish()
        })
    });

    let value = object_value(8, 7);
    group.bench_function("object_8_keys", |bencher| {
        bencher.iter(|| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            black_box(&value).hash(&mut hasher);
            hasher.finish()
        })
    });

    group.finish();
}

/// `Value` is used as a `BTreeMap` key for aggregation group keys and as a
/// `HashSet` member for `In` filters, so both orderings show up at runtime.
fn bench_collections(criterion: &mut Criterion) {
    const COUNT: usize = 10_000;

    let values: Vec<Value> = (0..COUNT)
        .map(|index| object_value(8, index as u64))
        .collect();

    let mut group = criterion.benchmark_group("value_collections");
    group.sample_size(20);

    group.bench_function("sort_10k_objects", |bencher| {
        bencher.iter_batched(
            || values.clone(),
            |mut items| {
                items.sort();
                items
            },
            BatchSize::LargeInput,
        )
    });

    group.bench_function("btree_map_insert_10k_objects", |bencher| {
        bencher.iter(|| {
            let mut map: BTreeMap<Value, usize> = BTreeMap::new();
            for (index, value) in values.iter().enumerate() {
                map.insert(value.clone(), index);
            }
            map.len()
        })
    });

    group.bench_function("hash_set_insert_10k_objects", |bencher| {
        bencher.iter(|| {
            let mut set: HashSet<Value> = HashSet::with_capacity(COUNT);
            for value in &values {
                set.insert(value.clone());
            }
            set.len()
        })
    });

    group.finish();
}

fn bench_api_name(criterion: &mut Criterion) {
    let mut group = criterion.benchmark_group("api_name");

    group.bench_function("new_valid", |bencher| {
        bencher.iter(|| ApiName::new(black_box("customer_order_line")))
    });

    group.bench_function("new_unchecked", |bencher| {
        bencher.iter(|| ApiName::new_unchecked(black_box("customer_order_line")))
    });

    // The backend keys records by `Arc<str>`, so name construction is on the hot
    // path of every record lookup.
    group.bench_function("from_static", |bencher| {
        bencher.iter(|| ApiName::from(black_box("customer_order_line")))
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_ordering,
    bench_hashing,
    bench_collections,
    bench_api_name
);
criterion_main!(benches);
