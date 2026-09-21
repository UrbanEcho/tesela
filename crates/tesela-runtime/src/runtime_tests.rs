use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tesela_backend::{
    ActionRequest, Backend, BindingsBuilder, ExecutionEvent, ExecutionObserver, FnActionExecutor,
    MemoryBackend, Mutation, Query,
};
use tesela_core::{ApiName, DataType, Error, ErrorCode, ExecutionContext, Key, Principal, Value};
use tesela_ir::{
    ActionEffects, ActionResult, ActionType, ApprovalPolicy, Concurrency, ObjectType, Property,
    Spec,
};

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
fn runtime() -> Result<(Arc<Runtime>, Arc<MemoryBackend>), Error> {
    let backend = MemoryBackend::new();
    let mut bindings = BindingsBuilder::new();
    bindings.bind_object(name("customer"), backend.clone());
    Ok((
        Runtime::builder(spec(), bindings.build(), Arc::new(AllowAllPolicy)).build()?,
        backend,
    ))
}
fn values(id: &'static str) -> BTreeMap<ApiName, Value> {
    [
        (name("id"), Value::string(id)),
        (name("tenant_id"), Value::string("acme")),
    ]
    .into()
}

#[test]
fn native_upsert_is_revision_safe() -> Result<(), Error> {
    let (runtime, _) = runtime()?;
    let created = runtime
        .mutate(
            &context(),
            &name("customer"),
            Mutation::Upsert {
                values: values("one"),
                expected_revision: None,
                preconditions: Vec::new(),
            },
        )?
        .output
        .record
        .expect("record");
    let stale = runtime.mutate(
        &context(),
        &name("customer"),
        Mutation::Upsert {
            values: values("one"),
            expected_revision: Some(tesela_ir::Revision::new("stale")),
            preconditions: Vec::new(),
        },
    );
    assert!(stale.is_err_and(|error| error.is(ErrorCode::StaleRevision)));
    let current = runtime
        .get(&context(), &name("customer"), &Key::string("one"))?
        .output;
    assert_eq!(current.revision, created.revision);
    Ok(())
}

#[test]
fn mutation_preconditions_fail_without_starting_the_write() -> Result<(), Error> {
    let (runtime, _) = runtime()?;
    runtime.mutate(
        &context(),
        &name("customer"),
        Mutation::Create {
            values: values("one"),
        },
    )?;
    let denied = runtime.mutate(
        &context(),
        &name("customer"),
        Mutation::Update {
            primary_key: Key::string("one"),
            values: [(name("tenant_id"), Value::string("globex"))].into(),
            expected_revision: None,
            preconditions: vec![tesela_ir::Filter::eq(
                name("tenant_id"),
                Value::string("globex"),
            )],
        },
    );
    assert!(denied.is_err_and(|error| {
        error.is(ErrorCode::PreconditionFailed)
            && error.side_effect() == tesela_core::SideEffectStatus::NotStarted
    }));
    let record = runtime
        .get(&context(), &name("customer"), &Key::string("one"))?
        .output;
    assert_eq!(record.get("tenant_id"), Some(&Value::string("acme")));
    Ok(())
}

#[test]
fn omitted_query_limit_is_bounded() -> Result<(), Error> {
    let (runtime, _) = runtime()?;
    for id in ["one", "two"] {
        runtime.mutate(
            &context(),
            &name("customer"),
            Mutation::Create { values: values(id) },
        )?;
    }
    let page = runtime
        .search(&context(), &name("customer"), Query::default())?
        .output;
    assert_eq!(page.records.len(), 2);
    Ok(())
}

#[test]
fn spec_replacement_is_atomic_and_bumps_generation() -> Result<(), Error> {
    let (runtime, _) = runtime()?;
    let before = runtime.ontology().generation();
    let mut invalid = spec();
    invalid.object_types[0].primary_key = name("missing");
    assert!(runtime.apply_spec(invalid).is_err());
    assert_eq!(runtime.ontology().generation(), before);
    runtime.apply_spec(spec())?;
    assert_eq!(runtime.ontology().generation(), before + 1);
    Ok(())
}

#[test]
fn capability_catalogs_are_versioned_and_exact_names_win() -> Result<(), Error> {
    let (runtime, _) = runtime()?;
    let catalog = runtime.capabilities(&context(), "customer.search")?;
    assert_eq!(
        catalog.search("customer.search", 1)[0].name,
        "customer.search"
    );
    assert_eq!(catalog.generation, runtime.ontology().generation());
    Ok(())
}

struct PanickingObserver;
impl ExecutionObserver for PanickingObserver {
    fn observe(&self, _event: &ExecutionEvent) {
        panic!("observer failures are isolated")
    }
}

#[test]
fn observer_panic_cannot_fail_a_committed_mutation() -> Result<(), Error> {
    let backend = MemoryBackend::new();
    let mut bindings = BindingsBuilder::new();
    bindings.bind_object(name("customer"), backend.clone());
    let runtime = Runtime::builder(spec(), bindings.build(), Arc::new(AllowAllPolicy))
        .observer(Arc::new(PanickingObserver))
        .build()?;
    let result = runtime.mutate(
        &context(),
        &name("customer"),
        Mutation::Create {
            values: values("one"),
        },
    )?;
    assert!(result.output.record.is_some());
    assert!(
        backend
            .get(&name("customer"), &Key::string("one"))?
            .is_some()
    );
    Ok(())
}

#[test]
fn action_input_and_idempotency_are_checked_before_execution() -> Result<(), Error> {
    let mut spec = spec();
    spec.upsert_action(ActionType {
        api_name: name("notify"),
        display: None,
        description: None,
        subject: None,
        input_schema: Some(Value::new(serde_json::json!({
            "type": "object",
            "properties": {"email": {"type": "string"}},
            "required": ["email"]
        }))),
        output_schema: None,
        risk_level: tesela_core::RiskLevel::Low,
        effects: ActionEffects {
            reads: false,
            writes: false,
            external: true,
            idempotent: true,
        },
        reads: Vec::new(),
        writes: Vec::new(),
        creates: Vec::new(),
        external_effects: vec![name("mail")],
        approval: ApprovalPolicy::Never,
        requires_revision: false,
        preconditions: Vec::new(),
        concurrency: Concurrency::Unrestricted,
        idempotency_key: Some("run_id".to_string()),
        deprecated_at: None,
    });
    let called = Arc::new(AtomicBool::new(false));
    let called_by_executor = Arc::clone(&called);
    let mut bindings = BindingsBuilder::new();
    bindings.bind_action(
        name("notify"),
        FnActionExecutor::new(move |_, _| {
            called_by_executor.store(true, Ordering::Release);
            Ok(ActionResult {
                status: "success".to_string(),
                output: None,
                error: None,
                run_id: None,
            })
        }),
    );
    let runtime = Runtime::builder(spec, bindings.build(), Arc::new(AllowAllPolicy)).build()?;
    let invalid = runtime.execute_action(
        &context(),
        ActionRequest {
            action: name("notify"),
            input: Value::new(serde_json::json!({"email": 7})),
            run_id: Some("one".to_string()),
            expected_revision: None,
            preconditions: Vec::new(),
        },
    );
    assert!(invalid.is_err_and(|error| error.is(ErrorCode::BadRequest)));
    assert!(!called.load(Ordering::Acquire));
    let missing_idempotency = runtime.execute_action(
        &context(),
        ActionRequest {
            action: name("notify"),
            input: Value::new(serde_json::json!({"email": "a@test"})),
            run_id: None,
            expected_revision: None,
            preconditions: Vec::new(),
        },
    );
    assert!(missing_idempotency.is_err_and(|error| error.is(ErrorCode::PreconditionFailed)));
    assert!(!called.load(Ordering::Acquire));
    Ok(())
}
