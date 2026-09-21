//! End-to-end throughput harness.
//!
//! Run through [`benchmarks/baseline.py`](../../../../benchmarks/baseline.py),
//! which records the numbers this prints. It measures the paths a host actually
//! calls — spec parse and serialize, search, get, upsert — so a change to any
//! of them shows up here rather than in a microbenchmark nobody reads.

use std::collections::BTreeMap;
use std::hint::black_box;
use std::time::{Duration, Instant};

use tesela::backend::BindingsBuilder;
use tesela::dev::MemoryBackend;
use tesela::model::{ApiName, DataType, Key, Spec, Value};
use tesela::policy::{AllowAllPolicy, ExecutionContext, Principal};
use tesela::prelude::{Error, Mutation, Query, Runtime};

#[derive(tesela::ObjectType)]
#[tesela(primary_key = "id")]
struct Customer {
    id: String,
    score: i64,
}

const ROWS: i64 = 100;

fn measure<T, F>(name: &str, count: usize, mut work: F) -> Result<(), Error>
where
    F: FnMut() -> Result<T, Error>,
{
    // One warm-up pass, so the first allocation does not land in the sample.
    black_box(work()?);
    let start = Instant::now();
    for _ in 0..count {
        black_box(work()?);
    }
    let elapsed: Duration = start.elapsed();
    println!(
        "{name}: {:.3} us/op ({count} ops)",
        elapsed.as_secs_f64() * 1_000_000.0 / count as f64
    );
    Ok(())
}

fn main() -> Result<(), Error> {
    let mut spec = Spec::default();
    spec.upsert_object_type(Customer::tesela_object_type());

    let customer = Customer::tesela_object_type().api_name;
    let backend = MemoryBackend::new();
    let mut bindings = BindingsBuilder::new();
    bindings.bind_object(customer.clone(), backend.clone());
    let runtime = Runtime::builder(
        spec.clone(),
        bindings.build(),
        std::sync::Arc::new(AllowAllPolicy),
    )
    .build()?;
    let context = ExecutionContext::trusted(Principal::new("bench"), "benchmark")?;

    for id in 0..ROWS {
        let mut values = BTreeMap::new();
        values.insert(ApiName::new("id")?, Value::string(id.to_string()));
        values.insert(ApiName::new("score")?, Value::integer(id));
        runtime.mutate(&context, &customer, Mutation::Create { values })?;
    }

    let json = spec.to_json_string()?;
    let mut update = BTreeMap::new();
    update.insert(ApiName::new("id")?, Value::string("0"));
    update.insert(ApiName::new("score")?, Value::integer(1));

    let row = Customer {
        id: "0".to_string(),
        score: 0,
    };
    black_box((row.id.as_str(), row.score, DataType::String));
    black_box(backend.len(&customer)?);

    measure("spec.parse", 10_000, || Spec::parse(json.as_bytes()))?;
    measure("spec.serialize", 10_000, || spec.to_json_string())?;
    measure("search.all", 10_000, || {
        runtime.search(&context, &customer, Query::default())
    })?;
    measure("get", 10_000, || {
        runtime.get(&context, &customer, &Key::string("0"))
    })?;
    measure("mutate.upsert", 10_000, || {
        runtime.mutate(
            &context,
            &customer,
            Mutation::Upsert {
                values: update.clone(),
                expected_revision: None,
                preconditions: Vec::new(),
            },
        )
    })?;
    Ok(())
}
