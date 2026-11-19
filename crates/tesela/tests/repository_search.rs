use std::sync::{Arc, Mutex};

use tesela::backend::{Native, ObjectBinding, native};
use tesela::prelude::*;
use tesela::raw::{ApiName, Backend as RawBackend, Key, Query as RawQuery, Record, conformance};
use tesela::{Page, Query, count, create, query};

#[derive(Debug, Clone, PartialEq, Object)]
#[tesela(key = id)]
struct Customer {
    id: String,
    region: Option<String>,
    spend: i64,
}

fn customers(count: usize) -> Vec<Customer> {
    (0..count)
        .map(|index| Customer {
            id: format!("c-{index:03}"),
            region: (index % 7 != 0).then(|| ["north", "south", "east"][index % 3].to_owned()),
            spend: i64::try_from((index * 37) % 101).expect("small"),
        })
        .collect()
}

fn context() -> Result<Context, Error> {
    Context::builder("test").request_id("repository").build()
}

#[test]
fn a_repository_scan_matches_the_memory_backend_for_every_query_shape() -> Result<(), Error> {
    let data = customers(60);
    let memory = Runtime::builder()
        .object::<Customer>(MemoryBackend::new())
        .policy(tesela::policy::allow_all)
        .build()?;
    let rows = data.clone();
    let repository = Runtime::builder()
        .object::<Customer>(
            backend::<Customer>()
                .scan(move |_| Ok(rows.clone()))
                .build(),
        )
        .policy(tesela::policy::allow_all)
        .build()?;
    let context = context()?;
    for customer in &data {
        memory.run(create(customer.clone()), &context)?;
    }
    let plans = || {
        vec![
            query::<Customer>().limit(7),
            query::<Customer>()
                .filter(Customer::region().eq("north"))
                .limit(50),
            query::<Customer>()
                .filter(
                    Customer::spend()
                        .gt(40)
                        .and(Customer::region().ends_with("th")),
                )
                .order_by(Customer::spend().desc())
                .limit(5),
            query::<Customer>()
                .order_by(Customer::spend().asc())
                .order_by(Customer::id().desc())
                .limit(9),
            query::<Customer>().filter(Customer::spend().gt(1000)),
        ]
    };
    for (left, right) in plans().into_iter().zip(plans()) {
        let expected = walk(&memory, left, &context)?;
        let actual = walk(&repository, right, &context)?;
        assert_eq!(actual, expected);
    }
    Ok(())
}

/// Follow `next_cursor` to the end, returning every page.
fn walk(
    runtime: &Runtime,
    plan: Query<Customer>,
    context: &Context,
) -> Result<Vec<Page<Customer>>, Error> {
    let mut pages = Vec::new();
    let mut next = plan.clone();
    loop {
        let page: Page<Customer> = runtime.run(next, context)?;
        let cursor = page.next_cursor.clone();
        pages.push(page);
        match cursor {
            Some(cursor) => next = plan.clone().after(cursor),
            None => return Ok(pages),
        }
        assert!(pages.len() < 100, "a cursor walk must terminate");
    }
}

/// A native backend that honours filter, sort and limit over fixed rows.
struct Ordered(Vec<Customer>);

impl Native<Customer> for Ordered {
    fn search(&self, _: &Context, query: &RawQuery) -> Result<Vec<Customer>, Error> {
        let records = self
            .0
            .iter()
            .map(|customer| {
                Ok(Record::new(
                    Key::Str(customer.id.as_str().into()),
                    customer.encode()?,
                ))
            })
            .collect::<Result<Vec<_>, Error>>()?;
        tesela::raw::execute_residual(
            &records,
            query,
            "ordered",
            tesela::raw::ScanOrder::Arbitrary,
        )?
        .records
        .into_iter()
        .map(Customer::decode)
        .collect()
    }
}

/// A native backend that ignores the requested order.
struct Unordered(Vec<Customer>);

impl Native<Customer> for Unordered {
    fn search(&self, _: &Context, query: &RawQuery) -> Result<Vec<Customer>, Error> {
        let limit = usize::try_from(query.limit.unwrap_or(i32::MAX)).unwrap_or(0);
        Ok(self.0.iter().rev().take(limit).cloned().collect())
    }
}

#[test]
fn a_native_backend_pages_by_cursor_and_reports_the_end() -> Result<(), Error> {
    let context = context()?;
    for (count, pages) in [(10, 3), (8, 2)] {
        let runtime = Runtime::builder()
            .object::<Customer>(native(Ordered(customers(count))))
            .policy(tesela::policy::allow_all)
            .build()?;
        let walked = walk(&runtime, query::<Customer>().limit(4), &context)?;
        assert_eq!(walked.len(), pages);
        assert!(walked.last().is_some_and(|page| page.next_cursor.is_none()));
        let ids: Vec<_> = walked
            .iter()
            .flat_map(|page| page.items.iter().map(|item| item.id.clone()))
            .collect();
        let expected: Vec<_> = customers(count).into_iter().map(|item| item.id).collect();
        assert_eq!(ids, expected);
    }
    Ok(())
}

#[test]
fn a_native_backend_out_of_keyset_order_is_a_contract_error() -> Result<(), Error> {
    let runtime = Runtime::builder()
        .object::<Customer>(native(Unordered(customers(10))))
        .policy(tesela::policy::allow_all)
        .build()?;
    let error = runtime
        .run(query::<Customer>().limit(4), &context()?)
        .expect_err("rows out of order are rejected");
    assert!(error.to_string().contains("keyset order"), "{error}");
    Ok(())
}

#[test]
fn fallback_aggregation_matches_native_aggregation() -> Result<(), Error> {
    let data = customers(500);
    let memory = Runtime::builder()
        .object::<Customer>(MemoryBackend::new())
        .policy(tesela::policy::allow_all)
        .build()?;
    let rows = data.clone();
    let repository = Runtime::builder()
        .object::<Customer>(
            backend::<Customer>()
                .scan(move |_| Ok(rows.clone()))
                .build(),
        )
        .policy(tesela::policy::allow_all)
        .build()?;
    let context = context()?;
    for customer in &data {
        memory.run(create(customer.clone()), &context)?;
    }
    let plan = || {
        query::<Customer>().group_by(Customer::region()).agg([
            count("n"),
            Customer::spend().sum("total"),
            Customer::spend().max("top"),
        ])
    };
    let expected = memory.run(plan(), &context)?;
    let actual = repository.run(plan(), &context)?;
    assert_eq!(actual.groups, expected.groups);
    assert_eq!(actual.groups.len(), 4);
    Ok(())
}

#[test]
fn the_byte_budget_still_applies_when_it_is_set() -> Result<(), Error> {
    let rows = customers(40);
    let runtime = Runtime::builder()
        .object::<Customer>(
            backend::<Customer>()
                .scan(move |_| Ok(rows.clone()))
                .build(),
        )
        .policy(tesela::policy::allow_all)
        .build()?;
    let limited = Context::builder("test")
        .request_id("budget")
        .budget(tesela::Budget {
            bytes: Some(64),
            ..tesela::Budget::default()
        })
        .build()?;
    let error = runtime
        .run(query::<Customer>().limit(40), &limited)
        .expect_err("40 rows exceed 64 bytes");
    assert_eq!(error.code(), tesela::ErrorCode::BudgetExhausted);
    assert_eq!(
        runtime.run(query::<Customer>(), &context()?)?.items.len(),
        40
    );
    Ok(())
}

/// Rows of the backend conformance kit: optional integer sort properties.
#[derive(Debug, Clone, PartialEq, Object)]
#[tesela(key = id)]
struct Probe {
    id: String,
    a: Option<i64>,
    b: Option<i64>,
}

type Shared = Arc<Mutex<Vec<Probe>>>;

fn probe_rows(records: &[Record]) -> Result<Vec<Probe>, Error> {
    records.iter().cloned().map(Probe::decode).collect()
}

fn probe_record(probe: &Probe) -> Result<Record, Error> {
    Ok(Record::new(
        Key::Str(probe.id.as_str().into()),
        probe.encode()?,
    ))
}

fn run_cursor_suite(backend: &dyn RawBackend, shared: &Shared) -> Result<(), Error> {
    let seed = |_: &ApiName, records: &[Record]| {
        *shared.lock().expect("probe rows") = probe_rows(records)?;
        Ok(())
    };
    conformance::verify_cursors(
        backend,
        &conformance::CursorFixture {
            object: ApiName::new_unchecked("probe"),
            primary_key: ApiName::new_unchecked("id"),
            seed: &seed,
        },
    )
}

/// A native backend over shared rows that honours filter, sort and limit.
struct SharedNative(Shared);

impl Native<Probe> for SharedNative {
    fn search(&self, _: &Context, query: &RawQuery) -> Result<Vec<Probe>, Error> {
        let records = self
            .0
            .lock()
            .expect("probe rows")
            .iter()
            .map(probe_record)
            .collect::<Result<Vec<_>, Error>>()?;
        tesela::raw::execute_residual(&records, query, "shared", tesela::raw::ScanOrder::Arbitrary)?
            .records
            .into_iter()
            .map(Probe::decode)
            .collect()
    }
}

#[test]
fn the_native_adapter_passes_the_cursor_walk_conformance_suite() -> Result<(), Error> {
    let shared = Shared::default();
    let adapter = ObjectBinding::<Probe>::into_raw(native(SharedNative(shared.clone())));
    run_cursor_suite(adapter.as_ref(), &shared)
}

#[test]
fn the_repository_scan_adapter_passes_the_cursor_walk_conformance_suite() -> Result<(), Error> {
    let shared = Shared::default();
    let (scan, create, delete) = (shared.clone(), shared.clone(), shared.clone());
    let repository = backend::<Probe>()
        .scan(move |_| Ok(scan.lock().expect("probe rows").clone()))
        .create(move |_, probe| {
            create.lock().expect("probe rows").push(probe.clone());
            Ok(probe)
        })
        .delete(move |_, id| {
            delete
                .lock()
                .expect("probe rows")
                .retain(|probe| &probe.id != id);
            Ok(())
        })
        .build();
    let adapter = ObjectBinding::<Probe>::into_raw(repository);
    run_cursor_suite(adapter.as_ref(), &shared)
}
