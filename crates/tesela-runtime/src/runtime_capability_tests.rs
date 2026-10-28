use std::sync::Arc;

use serde_json::json;
use tesela_backend::{BindingsBuilder, MemoryBackend, PolicyEngine};
use tesela_core::{ApiName, DataType, Error, ExecutionContext, Principal, RiskLevel};
use tesela_ir::{ObjectType, Property, Spec};

use crate::capability::{Capability, CapabilityCatalog, ToolExposure};
use crate::{AllowAllPolicy, Runtime};

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
