use tesela::backend::{Native, native};
use tesela::prelude::*;
use tesela::raw::Query as RawQuery;
use tesela::{Page, count, create, query};

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
                .limit(5)
                .offset(3),
            query::<Customer>()
                .order_by(Customer::spend().asc())
                .order_by(Customer::id().desc())
                .limit(9)
                .offset(20),
            query::<Customer>().filter(Customer::spend().gt(1000)),
        ]
    };
    for (left, right) in plans().into_iter().zip(plans()) {
        let expected: Page<Customer> = memory.run(left, &context)?;
        let actual: Page<Customer> = repository.run(right, &context)?;
        assert_eq!(actual, expected);
    }
    Ok(())
}

struct Slice(Vec<Customer>);

impl Native<Customer> for Slice {
    fn search(&self, _: &Context, query: &RawQuery) -> Result<Vec<Customer>, Error> {
        let offset = usize::try_from(query.offset.unwrap_or(0)).unwrap_or(0);
        let limit = usize::try_from(query.limit.unwrap_or(i32::MAX)).unwrap_or(0);
        Ok(self.0.iter().skip(offset).take(limit).cloned().collect())
    }
}

#[test]
fn a_native_backend_reports_a_cursor_only_while_rows_remain() -> Result<(), Error> {
    let context = context()?;
    for (count, expected) in [
        (10, [Some("4"), Some("8"), None]),
        (8, [Some("4"), None, None]),
    ] {
        let runtime = Runtime::builder()
            .object::<Customer>(native(Slice(customers(count))))
            .policy(tesela::policy::allow_all)
            .build()?;
        let mut cursors = Vec::new();
        for page in 0..3 {
            let result = runtime.run(query::<Customer>().limit(4).offset(page * 4), &context)?;
            cursors.push(result.next_cursor);
            assert!(result.items.len() <= 4);
        }
        let cursors: Vec<_> = cursors.iter().map(Option::as_deref).collect();
        assert_eq!(cursors, expected);
    }
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
