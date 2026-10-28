use std::sync::Arc;

use serde_json::json;
use tesela_backend::{BindingsBuilder, DenyAllPolicy, MemoryBackend, PolicyEngine};
use tesela_core::{ApiName, DataType, Error, ExecutionContext, Principal, RiskLevel, Value};
use tesela_ir::{ObjectType, Property, Spec};

use crate::capability::{Capability, CapabilityCatalog, CapabilityInvocation, ToolExposure};
use crate::{AllowAllPolicy, InvocationOutput, Runtime};

fn name(value: &'static str) -> ApiName {
    ApiName::from(value)
}

fn property(value: &'static str) -> Property {
    Property {
        api_name: name(value),
        display: None,
        description: None,
        data_type: DataType::String,
        nullable: None,
        default: None,
        allowed_values: None,
        tags: Vec::new(),
    }
}

fn spec() -> Spec {
    let mut spec = Spec::default();
    spec.upsert_object_type(ObjectType {
        api_name: name("customer"),
        display: None,
        description: None,
        primary_key: name("id"),
        properties: vec![property("id"), property("tenant_id")],
        traits: Vec::new(),
        tags: Vec::new(),
        deprecated_at: None,
    });
    spec
}

fn context() -> ExecutionContext {
    ExecutionContext::trusted(Principal::new("test"), "test").expect("context")
}

fn runtime_with(policy: Arc<dyn PolicyEngine>) -> Result<Arc<Runtime>, Error> {
    let mut bindings = BindingsBuilder::new();
    bindings.bind_object(name("customer"), MemoryBackend::new());
    Runtime::builder(spec(), bindings.build(), policy).build()
}

fn catalog_for(runtime: &Runtime, task: &str) -> CapabilityCatalog {
    runtime.capabilities(&context(), task).expect("catalog")
}

fn find<'a>(catalog: &'a CapabilityCatalog, name: &str) -> &'a Capability {
    catalog
        .capabilities
        .iter()
        .find(|capability| capability.name == name)
        .expect("capability in catalog")
}

fn invocation(
    catalog: &CapabilityCatalog,
    capability: &str,
    input: serde_json::Value,
) -> CapabilityInvocation {
    CapabilityInvocation {
        generation: catalog.generation,
        fingerprint: catalog.fingerprint.clone(),
        capability: capability.to_string(),
        input: Value::new(input),
        expected_revision: None,
        run_id: None,
        retain_results: false,
    }
}

fn create(runtime: &Runtime, catalog: &CapabilityCatalog, id: &str) -> Result<(), Error> {
    let input = json!({"values": {"id": id, "tenant_id": "acme"}});
    runtime
        .invoke(&context(), invocation(catalog, "customer.create", input))
        .map(|_| ())
}

#[test]
fn exposure_matches_whole_tokens_and_operation_verbs() -> Result<(), Error> {
    let runtime = runtime_with(Arc::new(AllowAllPolicy))?;
    let exposure =
        |task: &str, capability: &str| find(&catalog_for(&runtime, task), capability).exposure;
    assert_eq!(
        exposure("search customer", "customer.search"),
        ToolExposure::Direct
    );
    assert_eq!(
        exposure("create customer", "customer.create"),
        ToolExposure::Direct
    );
    assert_eq!(
        exposure("create customer", "customer.update"),
        ToolExposure::ProgramOnly
    );
    assert_eq!(
        exposure("search customer", "customer.get"),
        ToolExposure::Discoverable
    );
    // `customers` is a different token from `customer`; it must not promote.
    assert_eq!(
        exposure("list customers", "customer.search"),
        ToolExposure::Discoverable
    );
    assert_eq!(
        exposure("cust search", "customer.search"),
        ToolExposure::Discoverable
    );
    Ok(())
}

#[test]
fn mutation_metadata_and_input_schemas_are_derived() -> Result<(), Error> {
    let runtime = runtime_with(Arc::new(AllowAllPolicy))?;
    let catalog = catalog_for(&runtime, "");

    let delete = find(&catalog, "customer.delete");
    assert_eq!(delete.risk, RiskLevel::High);
    assert!(delete.annotations.destructive);
    assert!(delete.annotations.idempotent);
    assert!(delete.requires_revision);

    let create = find(&catalog, "customer.create");
    assert_eq!(create.risk, RiskLevel::Medium);
    assert!(!create.annotations.destructive);
    assert!(!create.annotations.idempotent);
    assert!(!create.requires_revision);

    let search = find(&catalog, "customer.search");
    assert!(search.annotations.read_only);
    assert!(!search.annotations.open_world);
    assert!(search.input_schema.0["properties"].get("filter").is_some());

    let get = find(&catalog, "customer.get");
    assert_eq!(get.input_schema.0["required"], json!(["key"]));

    let values = &create.input_schema.0["properties"]["values"];
    assert!(values["properties"].get("id").is_some());
    assert!(values["properties"].get("tenant_id").is_some());
    assert_eq!(values["additionalProperties"], json!(false));
    Ok(())
}

#[test]
fn denied_capabilities_are_hidden_and_not_invocable() -> Result<(), Error> {
    let runtime = runtime_with(Arc::new(DenyAllPolicy))?;
    let catalog = catalog_for(&runtime, "");
    assert!(!catalog.capabilities.is_empty());
    assert!(
        catalog
            .capabilities
            .iter()
            .all(|capability| capability.exposure == ToolExposure::Hidden)
    );
    let error = runtime
        .invoke(
            &context(),
            invocation(&catalog, "customer.search", json!({})),
        )
        .expect_err("hidden capability is not invocable");
    assert!(error.message().contains("customer.search"));
    Ok(())
}

#[test]
fn stale_invocations_name_both_generations() -> Result<(), Error> {
    let runtime = runtime_with(Arc::new(AllowAllPolicy))?;
    let catalog = catalog_for(&runtime, "");
    let mut stale = invocation(&catalog, "customer.search", json!({}));
    stale.generation += 1;
    let error = runtime
        .invoke(&context(), stale)
        .expect_err("stale invocation is rejected");
    assert!(error.message().contains(&catalog.generation.to_string()));
    assert!(error.message().contains("Runtime::capabilities"));
    Ok(())
}

#[test]
fn invoke_routes_mutations_reads_and_searches() -> Result<(), Error> {
    let runtime = runtime_with(Arc::new(AllowAllPolicy))?;
    let catalog = catalog_for(&runtime, "");
    let created = runtime.invoke(
        &context(),
        invocation(
            &catalog,
            "customer.create",
            json!({"values": {"id": "one", "tenant_id": "acme"}}),
        ),
    )?;
    let InvocationOutput::Mutation(result) = created.output else {
        panic!("create returns a mutation result");
    };
    assert!(result.record.is_some());

    let read = runtime.invoke(
        &context(),
        invocation(&catalog, "customer.get", json!({"key": "one"})),
    )?;
    assert!(matches!(read.output, InvocationOutput::Record(_)));

    let searched = runtime.invoke(
        &context(),
        invocation(&catalog, "customer.search", json!({})),
    )?;
    let InvocationOutput::Page(page) = searched.output else {
        panic!("search returns a page");
    };
    assert_eq!(page.records.len(), 1);
    Ok(())
}

#[test]
fn retained_results_serialize_as_a_summary_only() -> Result<(), Error> {
    let runtime = runtime_with(Arc::new(AllowAllPolicy))?;
    let catalog = catalog_for(&runtime, "");
    create(&runtime, &catalog, "one")?;
    create(&runtime, &catalog, "two")?;
    let mut retained = invocation(&catalog, "customer.search", json!({}));
    retained.retain_results = true;
    let output = runtime.invoke(&context(), retained)?.output;
    let summary = serde_json::to_value(&output).expect("summary serializes");
    assert_eq!(summary["handle"]["handle"]["count"], json!(2));
    assert!(!summary.to_string().contains("acme"));
    Ok(())
}

#[test]
fn retain_results_is_rejected_for_reads_and_mutations() -> Result<(), Error> {
    let runtime = runtime_with(Arc::new(AllowAllPolicy))?;
    let catalog = catalog_for(&runtime, "");
    let mut read = invocation(&catalog, "customer.get", json!({"key": "one"}));
    read.retain_results = true;
    let error = runtime.invoke(&context(), read).expect_err("rejected");
    assert!(error.message().contains("retain_results"));
    Ok(())
}

#[test]
fn revisioned_mutations_require_an_expected_revision() -> Result<(), Error> {
    let runtime = runtime_with(Arc::new(AllowAllPolicy))?;
    let catalog = catalog_for(&runtime, "");
    let update = invocation(
        &catalog,
        "customer.update",
        json!({"primary_key": "one", "values": {"tenant_id": "beta"}}),
    );
    let error = runtime.invoke(&context(), update).expect_err("rejected");
    assert!(error.message().contains("requires expected_revision"));
    Ok(())
}
