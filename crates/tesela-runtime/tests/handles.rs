use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use tesela_backend::{
    Backend, BindingsBuilder, MemoryBackend, PolicyDecision, PolicyEngine, PolicyRequest,
};
use tesela_core::{ApiName, DataType, Error, ErrorCode, ExecutionContext, Key, Principal, Value};
use tesela_ir::{ObjectType, Property, Spec};
use tesela_runtime::capability::{CapabilityInvocation, Program, ProgramStep};
use tesela_runtime::{InvocationOutput, Runtime};

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

/// Allows everything.
struct AllowAll;
impl PolicyEngine for AllowAll {
    fn evaluate(&self, _request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error> {
        Ok(PolicyDecision {
            allow: true,
            ..PolicyDecision::default()
        })
    }
}

fn fixture() -> Result<(Arc<Runtime>, ExecutionContext), Error> {
    let mut spec = Spec::default();
    spec.upsert_object_type(ObjectType {
        api_name: name("member"),
        display: None,
        description: None,
        primary_key: name("id"),
        properties: vec![property("id"), property("team"), property("city")],
        traits: Vec::new(),
        tags: Vec::new(),
        deprecated_at: None,
    });
    let backend = MemoryBackend::new();
    let seed = ExecutionContext::trusted(Principal::new("seed"), "seed")?;
    for (id, team, city) in [
        ("a1", "red", "oslo"),
        ("a2", "red", "rome"),
        ("a3", "blue", "oslo"),
        ("a4", "blue", "oslo"),
    ] {
        let values: BTreeMap<_, _> = [
            (name("id"), Value::string(id)),
            (name("team"), Value::string(team)),
            (name("city"), Value::string(city)),
        ]
        .into();
        backend.create(&seed, &name("member"), Key::string(id), values)?;
    }
    let mut bindings = BindingsBuilder::new();
    bindings.bind_object(name("member"), backend);
    let runtime = Runtime::builder(spec, bindings.build(), Arc::new(AllowAll)).build()?;
    Ok((
        runtime,
        ExecutionContext::trusted(Principal::new("alice"), "turn-1")?,
    ))
}

fn invoke(
    runtime: &Runtime,
    context: &ExecutionContext,
    capability: &str,
    input: serde_json::Value,
    retain: bool,
) -> Result<InvocationOutput, Error> {
    let catalog = runtime.capabilities(context, "")?;
    Ok(runtime
        .invoke(
            context,
            CapabilityInvocation {
                generation: catalog.generation,
                fingerprint: catalog.fingerprint,
                capability: capability.to_string(),
                input: Value::new(input),
                expected_revision: None,
                run_id: None,
                retain_results: retain,
            },
        )?
        .output)
}

fn retain_red(runtime: &Runtime, context: &ExecutionContext) -> Result<String, Error> {
    let output = invoke(
        runtime,
        context,
        "member.search",
        json!({"filter": {"op": "eq", "field": "team", "value": "red"}}),
        true,
    )?;
    match output {
        InvocationOutput::Handle { handle, .. } => Ok(handle.id().to_string()),
        other => panic!("expected a handle, got {other:?}"),
    }
}

#[test]
fn retained_results_are_paged_through_a_handle() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let id = retain_red(&runtime, &context)?;
    assert_eq!(id, "h1");
    let InvocationOutput::Page(page) = invoke(
        &runtime,
        &context,
        "results.read",
        json!({"handle": id, "limit": 1, "fields": ["city"]}),
        false,
    )?
    else {
        panic!("expected a page");
    };
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.next_cursor.as_deref(), Some("1"));
    assert!(!page.records[0].values.contains_key("team"));
    Ok(())
}

#[test]
fn handles_are_counted_without_reading_rows() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let id = retain_red(&runtime, &context)?;
    let InvocationOutput::Aggregate(result) = invoke(
        &runtime,
        &context,
        "results.count_by",
        json!({"handle": id, "field": "city"}),
        false,
    )?
    else {
        panic!("expected an aggregate");
    };
    assert_eq!(result.groups.len(), 2);
    Ok(())
}

#[test]
fn a_handle_reference_feeds_the_next_capability() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let id = retain_red(&runtime, &context)?;
    let InvocationOutput::Page(page) = invoke(
        &runtime,
        &context,
        "member.search",
        json!({"filter": {
            "op": "in",
            "field": "id",
            "values": {"$handle": id, "field": "id"},
        }}),
        false,
    )?
    else {
        panic!("expected a page");
    };
    assert_eq!(page.records.len(), 2);
    Ok(())
}

#[test]
fn handles_do_not_cross_requests_and_can_be_released() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let id = retain_red(&runtime, &context)?;
    let other = ExecutionContext::trusted(Principal::new("alice"), "turn-2")?;
    let error = runtime.handle(&other, &id).expect_err("other request");
    assert_eq!(error.code(), ErrorCode::NotFound);
    runtime.release_handles(&context);
    assert!(runtime.handle(&context, &id).is_err());
    Ok(())
}

fn program(
    runtime: &Runtime,
    context: &ExecutionContext,
    steps: Vec<(&str, &str, serde_json::Value)>,
) -> Result<Program, Error> {
    let catalog = runtime.capabilities(context, "")?;
    Ok(Program {
        generation: catalog.generation,
        fingerprint: catalog.fingerprint,
        steps: steps
            .into_iter()
            .map(|(id, capability, input)| ProgramStep {
                id: id.to_string(),
                capability: capability.to_string(),
                input: Value::new(input),
                expected_revision: None,
            })
            .collect(),
        retain_results: false,
    })
}

#[test]
fn a_program_threads_one_steps_result_into_the_next() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let program = program(
        &runtime,
        &context,
        vec![
            (
                "red",
                "member.search",
                json!({"filter": {"op": "eq", "field": "team", "value": "red"}}),
            ),
            (
                "same",
                "member.search",
                json!({"filter": {
                    "op": "in",
                    "field": "id",
                    "values": {"$ref": "red", "field": "id"},
                }}),
            ),
        ],
    )?;
    let output = runtime.run_program(&context, program)?.output;
    assert_eq!(output.steps.len(), 2);
    let InvocationOutput::Page(page) = output.result else {
        panic!("expected a page");
    };
    assert_eq!(page.records.len(), 2);
    Ok(())
}

#[test]
fn an_invalid_program_runs_no_step() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let create = json!({"values": {"id": "z9", "team": "red", "city": "oslo"}});
    for bad in [
        ("b", "member.nope", json!({})),
        (
            "b",
            "member.search",
            json!({"filter": {"values": {"$ref": "later"}}}),
        ),
        ("a", "member.search", json!({})),
    ] {
        let program = program(
            &runtime,
            &context,
            vec![("a", "member.create", create.clone()), bad],
        )?;
        let error = runtime.run_program(&context, program).expect_err("invalid");
        assert!(matches!(
            error.code(),
            ErrorCode::NotFound | ErrorCode::BadRequest
        ));
    }
    let created = runtime.get(&context, &name("member"), &Key::string("z9"));
    assert!(created.is_err(), "no step of an invalid program may run");
    Ok(())
}

#[test]
fn a_failing_step_reports_how_many_completed() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let create = json!({"values": {"id": "z9", "team": "red", "city": "oslo"}});
    let program = program(
        &runtime,
        &context,
        vec![
            ("a", "member.create", create.clone()),
            ("b", "member.create", create),
        ],
    )?;
    let error = runtime
        .run_program(&context, program)
        .expect_err("duplicate");
    assert!(error.message().contains("after 1 of 2 step(s) completed"));
    Ok(())
}
