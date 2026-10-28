use serde_json::json;
use tesela::prelude::*;
use tesela::raw::{self, capability::CapabilityInvocation};

#[derive(Debug, Clone, PartialEq, Object)]
#[tesela(key = id)]
struct Customer {
    id: String,
    email: Option<String>,
    active: bool,
}

fn invocation(
    catalog: &raw::capability::CapabilityCatalog,
    capability: &str,
    input: serde_json::Value,
) -> CapabilityInvocation {
    CapabilityInvocation {
        generation: catalog.generation,
        fingerprint: catalog.fingerprint.clone(),
        capability: capability.to_string(),
        input: raw::Value::new(input),
        expected_revision: None,
        run_id: None,
        retain_results: false,
    }
}

#[test]
fn capabilities_invoke_through_checked_paths() -> Result<(), Error> {
    let runtime = Runtime::builder()
        .object::<Customer>(MemoryBackend::new())
        .policy(tesela::policy::allow_all)
        .build()?;
    let context = Context::builder("test").request_id("capability").build()?;
    let catalog = runtime.capabilities(&context, Some("create customer"))?;

    let created = runtime.invoke(
        &context,
        invocation(
            &catalog,
            "customer.create",
            json!({"values": {"id": "c-1", "email": "a@example.test", "active": true}}),
        ),
    )?;
    assert!(matches!(created.value, raw::InvocationOutput::Mutation(_)));

    let read = runtime.invoke(
        &context,
        invocation(&catalog, "customer.get", json!({"key": "c-1"})),
    )?;
    assert!(matches!(read.value, raw::InvocationOutput::Record(_)));

    let searched = runtime.invoke(&context, invocation(&catalog, "customer.search", json!({})))?;
    let raw::InvocationOutput::Page(page) = searched.value else {
        panic!("search returns a page");
    };
    assert_eq!(page.records.len(), 1);
    Ok(())
}

#[test]
fn stale_invocations_are_rejected_with_a_refresh_hint() -> Result<(), Error> {
    let runtime = Runtime::builder()
        .object::<Customer>(MemoryBackend::new())
        .policy(tesela::policy::allow_all)
        .build()?;
    let context = Context::builder("test").request_id("stale").build()?;
    let catalog = runtime.capabilities(&context, None)?;

    let mut stale = invocation(&catalog, "customer.search", json!({}));
    stale.generation += 1;
    let Err(error) = runtime.invoke(&context, stale) else {
        panic!("stale invocation is rejected");
    };
    assert!(error.message().contains("Runtime::capabilities"));
    Ok(())
}

#[test]
fn retained_results_are_queried_through_the_execution_scoped_handle() -> Result<(), Error> {
    let runtime = Runtime::builder()
        .object::<Customer>(MemoryBackend::new())
        .policy(tesela::policy::allow_all)
        .build()?;
    let context = Context::builder("test").request_id("retained").build()?;
    let catalog = runtime.capabilities(&context, None)?;
    for id in ["c-1", "c-2"] {
        runtime.invoke(
            &context,
            invocation(
                &catalog,
                "customer.create",
                json!({"values": {"id": id, "email": null, "active": true}}),
            ),
        )?;
    }

    let mut retained = invocation(&catalog, "customer.search", json!({}));
    retained.retain_results = true;
    let output = runtime.invoke(&context, retained)?.value;
    let raw::InvocationOutput::Handle { handle, .. } = output else {
        panic!("retained search returns a handle");
    };
    assert_eq!(handle.page(&context, 0, 10)?.len(), 2);
    Ok(())
}
