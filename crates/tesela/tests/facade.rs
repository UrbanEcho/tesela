use tesela::prelude::*;
use tesela::{Action, Link, Page, create, get, query, update};

#[derive(Debug, Clone, PartialEq, Object)]
#[tesela(key = id)]
struct Customer {
    id: String,
    email: Option<String>,
    active: bool,
}

#[derive(Link)]
#[tesela(from = Customer, to = Customer, cardinality = many)]
struct Referrals;

#[derive(Action)]
#[tesela(
    subject = Customer,
    returns = String,
    reads(Customer),
    writes(Customer),
    idempotent,
    concurrency = optimistic,
    approval = policy,
    risk = medium
)]
struct ChangeEmail {
    id: String,
    email: String,
}

#[test]
fn typed_plans_are_immutable_and_execute_through_runtime() -> Result<(), Error> {
    let backend = MemoryBackend::new();
    let runtime = Runtime::builder()
        .object::<Customer>(backend)
        .link::<Referrals, _>(|_, _| Ok(Vec::new()))
        .action::<ChangeEmail, _>(|_, command| Ok(command.email))
        .policy(tesela::policy::allow_all)
        .build()?;
    let context = Context::builder("test")
        .request_id("facade")
        .approve(ChangeEmail::definition().api_name)
        .build()?;
    let customer = Customer {
        id: "c-1".into(),
        email: Some("a@example.test".into()),
        active: true,
    };
    assert_eq!(runtime.run(create(customer.clone()), &context)?, customer);
    assert_eq!(
        runtime.run(get::<Customer>("c-1".into()), &context)?,
        Some(customer.clone())
    );
    let base = query::<Customer>();
    let filtered = base.clone().filter(Customer::active().eq(true)).limit(10);
    let page: Page<Customer> = runtime.run(filtered, &context)?;
    assert_eq!(page.items, vec![customer]);
    assert_eq!(runtime.run(base, &context)?.items.len(), 1);
    let optional_text = Customer::email()
        .starts_with("a")
        .and(Customer::email().ends_with("test"));
    assert_eq!(
        runtime
            .run(query::<Customer>().filter(optional_text), &context)?
            .items
            .len(),
        1
    );
    let traversal = tesela::traverse::<Referrals>("c-1".to_owned()).limit(2);
    assert!(runtime.run(traversal, &context)?.items.is_empty());
    let changed = runtime.run(
        update::<Customer>("c-1".into()).set(Customer::email(), Some("b@example.test".to_owned())),
        &context,
    )?;
    assert_eq!(changed.email.as_deref(), Some("b@example.test"));
    let action = runtime.run(
        ChangeEmail {
            id: "c-1".into(),
            email: "c@example.test".into(),
        },
        &context,
    )?;
    assert_eq!(action, "c@example.test");
    let definition = ChangeEmail::definition();
    assert!(definition.effects.idempotent);
    assert_eq!(definition.concurrency, tesela::raw::Concurrency::Optimistic);
    assert_eq!(definition.risk_level, tesela::raw::RiskLevel::Medium);
    Ok(())
}
