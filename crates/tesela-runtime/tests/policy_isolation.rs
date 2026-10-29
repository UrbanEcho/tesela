use std::collections::BTreeMap;
use std::sync::Arc;

use tesela_backend::{
    AggregateQuery, Aggregation, AggregationFunction, Backend, BindingsBuilder, MemoryBackend,
    PolicyDecision, PolicyEngine, PolicyRequest, TraversalQuery,
};
use tesela_core::{
    ApiName, DataType, Error, ErrorCode, ExecutionContext, FilterOp, Key, LinkCardinality,
    Operation, Principal, ResourceKind, Value,
};
use tesela_ir::{Filter, LinkType, ObjectType, Property, Spec};
use tesela_runtime::Runtime;

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

struct TenantPolicy;
impl PolicyEngine for TenantPolicy {
    fn evaluate(&self, request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error> {
        let mut decision = PolicyDecision {
            allow: true,
            ..PolicyDecision::default()
        };
        if request.resource_kind == ResourceKind::ObjectType {
            decision.row_filter = Some(Filter {
                op: FilterOp::Eq,
                field: Some(name("tenant_id")),
                value: Some(Value::string("acme")),
                values: Vec::new(),
                args: Vec::new(),
            });
            decision.redactions = vec![name("email")];
        }
        Ok(decision)
    }
}

fn fixture() -> Result<(Arc<Runtime>, ExecutionContext), Error> {
    let mut spec = Spec::default();
    spec.upsert_object_type(ObjectType {
        api_name: name("customer"),
        display: None,
        description: None,
        primary_key: name("id"),
        properties: vec![property("id"), property("tenant_id"), property("email")],
        traits: Vec::new(),
        tags: Vec::new(),
        deprecated_at: None,
    });
    let backend = MemoryBackend::new();
    let seed_context = ExecutionContext::trusted(Principal::new("seed"), "seed")?;
    for (id, tenant) in [("a", "acme"), ("g", "globex")] {
        let values: BTreeMap<_, _> = [
            (name("id"), Value::string(id)),
            (name("tenant_id"), Value::string(tenant)),
            (name("email"), Value::string(format!("{id}@test"))),
        ]
        .into();
        backend.create(&seed_context, &name("customer"), Key::string(id), values)?;
    }
    let mut bindings = BindingsBuilder::new();
    bindings.bind_object(name("customer"), backend);
    let runtime = Runtime::builder(spec, bindings.build(), Arc::new(TenantPolicy))
        .max_query_limit(10)
        .build()?;
    let context = ExecutionContext::trusted(Principal::new("alice"), "policy-test")?;
    Ok((runtime, context))
}

#[test]
fn get_enforces_row_filter_and_redaction() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let record = runtime
        .get(&context, &name("customer"), &Key::string("a"))?
        .output;
    assert!(!record.values.contains_key("email"));
    let denied = runtime.get(&context, &name("customer"), &Key::string("g"));
    assert!(denied.is_err_and(|error| error.is(ErrorCode::NotFound)));
    Ok(())
}

#[test]
fn aggregate_cannot_leak_redacted_group_keys() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let result = runtime
        .aggregate(
            &context,
            &name("customer"),
            AggregateQuery {
                group_by: vec![name("email")],
                aggregations: vec![Aggregation {
                    function: AggregationFunction::Count,
                    property: None,
                    alias: "total".to_string(),
                }],
                ..AggregateQuery::default()
            },
        )?
        .output;
    assert_eq!(result.groups[0].get("total"), Some(&Value::integer(1)));
    assert!(!result.groups[0].contains_key("email"));
    Ok(())
}

#[test]
fn mutation_row_scope_is_enforced_before_backend_execution() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let denied = runtime.mutate(
        &context,
        &name("customer"),
        tesela_backend::Mutation::Update {
            primary_key: Key::string("g"),
            values: [(name("email"), Value::string("changed@test"))].into(),
            expected_revision: None,
            preconditions: Vec::new(),
        },
    );
    assert!(denied.is_err_and(|error| error.is(ErrorCode::PolicyDenied)));
    Ok(())
}

struct TransformTenantPolicy;
impl PolicyEngine for TransformTenantPolicy {
    fn evaluate(&self, request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error> {
        let mut decision = PolicyDecision {
            allow: true,
            ..PolicyDecision::default()
        };
        if request.operation == Operation::Mutate {
            decision
                .transformations
                .insert(name("tenant_id"), Value::string("acme"));
        }
        Ok(decision)
    }
}

#[test]
fn policy_transformations_are_validated_and_persisted() -> Result<(), Error> {
    let mut spec = Spec::default();
    spec.upsert_object_type(ObjectType {
        api_name: name("customer"),
        display: None,
        description: None,
        primary_key: name("id"),
        properties: vec![property("id"), property("tenant_id"), property("email")],
        traits: Vec::new(),
        tags: Vec::new(),
        deprecated_at: None,
    });
    let backend = MemoryBackend::new();
    let mut bindings = BindingsBuilder::new();
    bindings.bind_object(name("customer"), backend);
    let runtime =
        Runtime::builder(spec, bindings.build(), Arc::new(TransformTenantPolicy)).build()?;
    runtime.mutate(
        &ExecutionContext::trusted(Principal::new("alice"), "transform")?,
        &name("customer"),
        tesela_backend::Mutation::Create {
            values: [
                (name("id"), Value::string("a")),
                (name("tenant_id"), Value::string("untrusted")),
                (name("email"), Value::string("a@test")),
            ]
            .into(),
        },
    )?;
    let stored = runtime
        .get(
            &ExecutionContext::trusted(Principal::new("alice"), "read")?,
            &name("customer"),
            &Key::string("a"),
        )?
        .output;
    assert_eq!(stored.get("tenant_id"), Some(&Value::string("acme")));
    Ok(())
}

#[test]
fn traversal_enforces_source_and_target_row_scopes() -> Result<(), Error> {
    let mut spec = Spec::default();
    for object in ["customer", "order"] {
        spec.upsert_object_type(ObjectType {
            api_name: name(object),
            display: None,
            description: None,
            primary_key: name("id"),
            properties: vec![
                property("id"),
                property("tenant_id"),
                property("customer_id"),
            ],
            traits: Vec::new(),
            tags: Vec::new(),
            deprecated_at: None,
        });
    }
    spec.upsert_link_type(LinkType {
        api_name: name("orders"),
        display: None,
        from: name("customer"),
        to: name("order"),
        cardinality: LinkCardinality::OneToMany,
        inverse: None,
        deprecated_at: None,
    });
    let backend = MemoryBackend::new();
    backend.bind_link(name("orders"), name("customer_id"));
    let seed_context = ExecutionContext::trusted(Principal::new("seed"), "seed")?;
    for (object, id, tenant, customer) in [
        ("customer", "a", "acme", "a"),
        ("customer", "g", "globex", "g"),
        ("order", "oa", "acme", "a"),
        ("order", "og", "globex", "a"),
    ] {
        backend.create(
            &seed_context,
            &name(object),
            Key::string(id),
            [
                (name("id"), Value::string(id)),
                (name("tenant_id"), Value::string(tenant)),
                (name("customer_id"), Value::string(customer)),
            ]
            .into(),
        )?;
    }
    let mut bindings = BindingsBuilder::new();
    bindings
        .bind_object(name("customer"), backend.clone())
        .bind_object(name("order"), backend.clone())
        .bind_link(name("orders"), backend);
    let runtime = Runtime::builder(spec, bindings.build(), Arc::new(TenantPolicy)).build()?;
    let context = ExecutionContext::trusted(Principal::new("alice"), "traverse")?;
    let visible = runtime
        .traverse(
            &context,
            &name("orders"),
            TraversalQuery {
                source_pk: Key::string("a"),
                filter: None,
                sort: Vec::new(),
                limit: None,
                offset: None,
                fields: None,
            },
        )?
        .output;
    assert_eq!(visible.records.len(), 1);
    assert_eq!(visible.records[0].primary_key, Key::string("oa"));
    let hidden_source = runtime.traverse(
        &context,
        &name("orders"),
        TraversalQuery {
            source_pk: Key::string("g"),
            filter: None,
            sort: Vec::new(),
            limit: None,
            offset: None,
            fields: None,
        },
    );
    assert!(hidden_source.is_err_and(|error| error.is(ErrorCode::NotFound)));
    Ok(())
}
