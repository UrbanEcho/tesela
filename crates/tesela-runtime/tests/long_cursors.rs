//! Cursor walks stay reachable for long sort values and long guarded runs.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde_json::json;
use tesela_backend::{
    Backend, BindingsBuilder, MemoryBackend, PolicyDecision, PolicyEngine, PolicyRequest, Query,
    Sort, SortDirection,
};
use tesela_core::{ApiName, DataType, Error, ExecutionContext, Key, Principal, Value};
use tesela_ir::{ObjectType, Property, Record, Spec};
use tesela_runtime::capability::{Program, ProgramStep};
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

/// With `guard` set, admits only `team == "red"` records.
struct Policy {
    guard: bool,
}
impl PolicyEngine for Policy {
    fn evaluate(&self, _request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error> {
        Ok(PolicyDecision {
            allow: true,
            ..PolicyDecision::default()
        })
    }
    fn guards_records(&self, _context: &ExecutionContext, _object: &ApiName) -> bool {
        self.guard
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

fn runtime(rows: &[(String, &str, String)], guard: bool) -> Result<Arc<Runtime>, Error> {
    let mut spec = Spec::default();
    spec.upsert_object_type(ObjectType {
        api_name: name("doc"),
        display: None,
        description: None,
        primary_key: name("id"),
        properties: vec![property("id"), property("team"), property("body")],
        traits: Vec::new(),
        tags: Vec::new(),
        deprecated_at: None,
    });
    let backend = MemoryBackend::new();
    let seed = ExecutionContext::trusted(Principal::new("seed"), "seed")?;
    for (id, team, body) in rows {
        let values: BTreeMap<_, _> = [
            (name("id"), Value::string(id.as_str())),
            (name("team"), Value::string(*team)),
            (name("body"), Value::string(body.as_str())),
        ]
        .into();
        backend.create(&seed, &name("doc"), Key::string(id.as_str()), values)?;
    }
    let mut bindings = BindingsBuilder::new();
    bindings.bind_object(name("doc"), backend);
    Runtime::builder(spec, bindings.build(), Arc::new(Policy { guard }))
        .max_query_limit(50)
        .build()
}

#[test]
fn a_sort_over_values_longer_than_any_token_bound_reaches_every_row() -> Result<(), Error> {
    let rows: Vec<_> = (0..3)
        .map(|index| {
            (
                format!("d{index}"),
                "red",
                format!("{}{index}", "x".repeat(40_000)),
            )
        })
        .collect();
    let runtime = runtime(&rows, false)?;
    let context = ExecutionContext::trusted(Principal::new("alice"), "long")?;
    let mut cursor = None;
    let mut seen = Vec::new();
    loop {
        let page = runtime
            .search(
                &context,
                &name("doc"),
                Query {
                    sort: vec![Sort {
                        property: name("body"),
                        direction: SortDirection::Asc,
                    }],
                    limit: Some(1),
                    cursor: cursor.take(),
                    ..Query::default()
                },
            )?
            .output;
        seen.extend(page.records.iter().map(|r| r.primary_key.to_string()));
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen, ["d0", "d1", "d2"]);
    Ok(())
}

#[test]
fn a_program_reference_survives_a_long_run_of_guard_rejected_pages() -> Result<(), Error> {
    // 100 denied rows sort before the three admitted ones, and a page of one
    // row: far more consecutive empty guarded pages than any stuck-walk bound.
    let mut rows: Vec<_> = (0..100)
        .map(|index| (format!("a{index:03}"), "blue", String::new()))
        .collect();
    rows.extend((0..3).map(|index| (format!("b{index}"), "red", String::new())));
    let runtime = runtime(&rows, true)?;
    let context = ExecutionContext::trusted(Principal::new("alice"), "guarded")?;
    let catalog = runtime.capabilities(&context, "")?;
    let program = Program {
        generation: catalog.generation,
        fingerprint: catalog.fingerprint,
        steps: vec![
            ProgramStep {
                id: "all".to_string(),
                capability: "doc.search".to_string(),
                input: Value::new(json!({"limit": 1})),
                expected_revision: None,
            },
            ProgramStep {
                id: "count".to_string(),
                capability: "doc.aggregate".to_string(),
                input: Value::new(json!({
                    "filter": {"op": "in", "field": "id", "values": {"$ref": "all", "field": "id"}},
                    "aggregations": [{"function": "count", "alias": "total"}],
                })),
                expected_revision: None,
            },
        ],
        retain_results: false,
    };
    let InvocationOutput::Aggregate(result) = runtime.run_program(&context, program)?.output.result
    else {
        panic!("expected an aggregate");
    };
    assert_eq!(result.groups[0].get("total"), Some(&Value::integer(3)));
    Ok(())
}
