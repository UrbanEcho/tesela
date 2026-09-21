use std::collections::BTreeMap;
use std::sync::Arc;

use tesela::backend::{Backend, BackendBuilder, BindingsBuilder};
use tesela::dev::MemoryBackend;
use tesela::model::{ApiName, DataType, ObjectType, Property, Spec, Value};
use tesela::policy::{AllowAllPolicy, ExecutionContext, Principal};
use tesela::query::Mutation;
use tesela::runtime::Runtime;

fn name(value: &'static str) -> ApiName {
    ApiName::from(value)
}
fn spec() -> Spec {
    let mut spec = Spec::default();
    spec.upsert_object_type(ObjectType {
        api_name: name("customer"),
        display: None,
        description: None,
        primary_key: name("id"),
        properties: vec![Property {
            api_name: name("id"),
            display: None,
            description: None,
            data_type: DataType::String,
            nullable: None,
            default: None,
            allowed_values: None,
            tags: Vec::new(),
        }],
        traits: Vec::new(),
        tags: Vec::new(),
        deprecated_at: None,
    });
    spec
}

#[test]
fn curated_facade_builds_and_runs() -> Result<(), tesela::error::Error> {
    let backend = MemoryBackend::new();
    let mut bindings = BindingsBuilder::new();
    bindings.bind_object(name("customer"), backend);
    let runtime = Runtime::builder(spec(), bindings.build(), Arc::new(AllowAllPolicy)).build()?;
    let context = ExecutionContext::trusted(Principal::new("test"), "facade")?;
    let values: BTreeMap<_, _> = [(name("id"), Value::string("one"))].into();
    runtime.mutate(&context, &name("customer"), Mutation::Create { values })?;
    assert_eq!(runtime.spec().object_types.len(), 1);
    Ok(())
}

#[test]
fn closure_backend_derives_capabilities() {
    let backend = BackendBuilder::new("repository")
        .get(|_, _| Ok(None))
        .build();
    assert!(backend.capabilities().get);
    assert!(!backend.capabilities().search);
}
