use std::collections::BTreeMap;
use std::sync::Arc;

use tesela_backend::{
    AggregateQuery, Aggregation, AggregationFunction, Backend, BindingsBuilder, MemoryBackend,
    PolicyDecision, PolicyEngine, PolicyRequest, Query,
};
use tesela_core::{ApiName, DataType, Error, ErrorCode, ExecutionContext, Key, Principal, Value};
use tesela_ir::{ObjectType, Property, Record, Spec};
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

/// Admits only records whose `team` is `red`, which no row filter states.
struct TeamGuard;
impl PolicyEngine for TeamGuard {
    fn evaluate(&self, _request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error> {
        Ok(PolicyDecision {
            allow: true,
            ..PolicyDecision::default()
        })
    }
    fn guards_records(&self, _context: &ExecutionContext, object: &ApiName) -> bool {
        object.as_ref() == "member"
    }
    fn admit_record(
        &self,
        _context: &ExecutionContext,
        _object: &ApiName,
        record: &Record,
    ) -> Result<bool, Error> {
        Ok(record.values.get("team") == Some(&Value::string("red")))
    }
}

fn fixture() -> Result<(Arc<Runtime>, ExecutionContext), Error> {
    let mut spec = Spec::default();
    spec.upsert_object_type(ObjectType {
        api_name: name("member"),
        display: None,
        description: None,
        primary_key: name("id"),
        properties: vec![property("id"), property("team")],
        traits: Vec::new(),
        tags: Vec::new(),
        deprecated_at: None,
    });
    let backend = MemoryBackend::new();
    let seed = ExecutionContext::trusted(Principal::new("seed"), "seed")?;
    // Seven blue records sort before the three red ones, so a small page
    // starts out empty and has to be refilled.
    for (id, team) in [
        ("a1", "blue"),
        ("a2", "blue"),
        ("a3", "blue"),
        ("a4", "blue"),
        ("a5", "blue"),
        ("a6", "blue"),
        ("a7", "blue"),
        ("b1", "red"),
        ("b2", "red"),
        ("b3", "red"),
    ] {
        let values: BTreeMap<_, _> = [
            (name("id"), Value::string(id)),
            (name("team"), Value::string(team)),
        ]
        .into();
        backend.create(&seed, &name("member"), Key::string(id), values)?;
    }
    let mut bindings = BindingsBuilder::new();
    bindings.bind_object(name("member"), backend);
    let runtime = Runtime::builder(spec, bindings.build(), Arc::new(TeamGuard))
        .max_query_limit(50)
        .build()?;
    Ok((
        runtime,
        ExecutionContext::trusted(Principal::new("alice"), "guard")?,
    ))
}

#[test]
fn search_refills_a_page_the_guard_thinned_out() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let page = runtime
        .search(
            &context,
            &name("member"),
            Query {
                limit: Some(2),
                ..Query::default()
            },
        )?
        .output;
    assert_eq!(page.records.len(), 2);
    assert!(
        page.records
            .iter()
            .all(|record| record.values.get("team") == Some(&Value::string("red")))
    );
    Ok(())
}

#[test]
fn get_refuses_a_record_the_guard_rejects() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    assert!(
        runtime
            .get(&context, &name("member"), &Key::string("b1"))
            .is_ok()
    );
    let denied = runtime.get(&context, &name("member"), &Key::string("a1"));
    assert!(denied.is_err_and(|error| error.is(ErrorCode::PolicyDenied)));
    Ok(())
}

#[test]
fn aggregate_counts_only_admitted_records() -> Result<(), Error> {
    let (runtime, context) = fixture()?;
    let result = runtime
        .aggregate(
            &context,
            &name("member"),
            AggregateQuery {
                aggregations: vec![Aggregation {
                    function: AggregationFunction::Count,
                    property: None,
                    alias: "total".to_string(),
                }],
                ..AggregateQuery::default()
            },
        )?
        .output;
    assert_eq!(result.groups[0].get("total"), Some(&Value::integer(3)));
    Ok(())
}
