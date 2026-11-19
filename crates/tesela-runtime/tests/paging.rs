//! Keyset paging in the runtime: handles, references, programs, aggregate
//! fallback and traversal all reach every row by following cursors.

use std::collections::BTreeSet;
use std::sync::Arc;

use serde_json::json;
use tesela_backend::{
    AggregateQuery, Aggregation, AggregationFunction, Backend, BindingsBuilder, MemoryBackend,
    PolicyDecision, PolicyEngine, PolicyRequest, Query, Sort, SortDirection, TraversalQuery,
};
use tesela_core::{
    ApiName, DataType, Error, ErrorCode, ExecutionContext, Key, LinkCardinality, Principal, Value,
};
use tesela_ir::{Cursor, LinkType, ObjectType, Property, Record, Spec};
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

/// Allows everything; with `guard` set, admits only `team == "red"` records.
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
    fn guards_records(&self, _context: &ExecutionContext, object: &ApiName) -> bool {
        self.guard && object.as_ref() == "member"
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

struct Fixture {
    runtime: Arc<Runtime>,
    context: ExecutionContext,
}

const TEAMS: [&str; 3] = ["red", "green", "blue"];

/// `members` rows `m0000..` cycle through three teams (so sorting by team ties
/// heavily); two of every three belong to owner `o1`, the rest to `o2`.
fn fixture(
    members: usize,
    max_limit: usize,
    retained: usize,
    guard: bool,
) -> Result<Fixture, Error> {
    let mut spec = Spec::default();
    spec.upsert_object_type(ObjectType {
        api_name: name("owner"),
        display: None,
        description: None,
        primary_key: name("id"),
        properties: vec![property("id")],
        traits: Vec::new(),
        tags: Vec::new(),
        deprecated_at: None,
    });
    spec.upsert_object_type(ObjectType {
        api_name: name("member"),
        display: None,
        description: None,
        primary_key: name("id"),
        properties: vec![property("id"), property("team"), property("owner")],
        traits: Vec::new(),
        tags: Vec::new(),
        deprecated_at: None,
    });
    spec.upsert_link_type(LinkType {
        api_name: name("members"),
        display: None,
        from: name("owner"),
        to: name("member"),
        cardinality: LinkCardinality::OneToMany,
        inverse: None,
        deprecated_at: None,
    });
    let backend = MemoryBackend::new();
    backend.bind_link(name("members"), name("owner"));
    let seed = ExecutionContext::trusted(Principal::new("seed"), "seed")?;
    for owner in ["o1", "o2"] {
        backend.create(
            &seed,
            &name("owner"),
            Key::string(owner),
            [(name("id"), Value::string(owner))].into(),
        )?;
    }
    for index in 0..members {
        let id = format!("m{index:04}");
        backend.create(
            &seed,
            &name("member"),
            Key::string(&id),
            [
                (name("id"), Value::string(&id)),
                (name("team"), Value::string(TEAMS[index % 3])),
                (
                    name("owner"),
                    Value::string(if index % 3 == 2 { "o2" } else { "o1" }),
                ),
            ]
            .into(),
        )?;
    }
    let mut bindings = BindingsBuilder::new();
    bindings
        .bind_object(name("owner"), backend.clone())
        .bind_object(name("member"), backend.clone())
        .bind_link(name("members"), backend);
    let runtime = Runtime::builder(spec, bindings.build(), Arc::new(Policy { guard }))
        .max_query_limit(max_limit)
        .max_retained_requests(retained)
        .build()?;
    Ok(Fixture {
        runtime,
        context: ExecutionContext::trusted(Principal::new("alice"), "turn-1")?,
    })
}

fn invocation(
    runtime: &Runtime,
    capability: &str,
    input: serde_json::Value,
    retain: bool,
) -> Result<CapabilityInvocation, Error> {
    let catalog = runtime.capabilities(
        &ExecutionContext::trusted(Principal::new("alice"), "cat")?,
        "",
    )?;
    Ok(CapabilityInvocation {
        generation: catalog.generation,
        fingerprint: catalog.fingerprint,
        capability: capability.to_string(),
        input: Value::new(input),
        expected_revision: None,
        run_id: None,
        retain_results: retain,
    })
}

fn invoke(
    fixture: &Fixture,
    capability: &str,
    input: serde_json::Value,
    retain: bool,
) -> Result<InvocationOutput, Error> {
    let call = invocation(&fixture.runtime, capability, input, retain)?;
    Ok(fixture.runtime.invoke(&fixture.context, call)?.output)
}

fn ids(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .map(|record| record.primary_key.to_string())
        .collect()
}

#[test]
fn a_reference_expands_every_handle_row_past_a_thousand_values() -> Result<(), Error> {
    let fixture = fixture(1_300, 2_000, 4, false)?;
    let InvocationOutput::Handle { handle, .. } =
        invoke(&fixture, "member.search", json!({}), true)?
    else {
        panic!("expected a handle");
    };
    assert_eq!(handle.len(), 1_300);
    let InvocationOutput::Aggregate(result) = invoke(
        &fixture,
        "member.aggregate",
        json!({
            "filter": {"op": "in", "field": "id", "values": {"$handle": handle.id(), "field": "id"}},
            "aggregations": [{"function": "count", "alias": "total"}],
        }),
        false,
    )?
    else {
        panic!("expected an aggregate");
    };
    assert_eq!(result.groups[0].get("total"), Some(&Value::integer(1_300)));
    Ok(())
}

#[test]
fn a_program_reference_follows_every_page_of_the_step() -> Result<(), Error> {
    let fixture = fixture(35, 10, 4, false)?;
    let catalog = fixture.runtime.capabilities(&fixture.context, "")?;
    let program = Program {
        generation: catalog.generation,
        fingerprint: catalog.fingerprint,
        steps: vec![
            ProgramStep {
                id: "all".to_string(),
                capability: "member.search".to_string(),
                input: Value::new(json!({"limit": 10})),
                expected_revision: None,
            },
            ProgramStep {
                id: "count".to_string(),
                capability: "member.aggregate".to_string(),
                input: Value::new(json!({
                    "filter": {"op": "in", "field": "id", "values": {"$ref": "all", "field": "id"}},
                    "aggregations": [{"function": "count", "alias": "total"}],
                })),
                expected_revision: None,
            },
        ],
        retain_results: false,
    };
    let InvocationOutput::Aggregate(result) = fixture
        .runtime
        .run_program(&fixture.context, program)?
        .output
        .result
    else {
        panic!("expected an aggregate");
    };
    assert_eq!(result.groups[0].get("total"), Some(&Value::integer(35)));
    Ok(())
}

#[test]
fn the_aggregate_fallback_counts_rows_past_one_page() -> Result<(), Error> {
    // 35 rows, a page of 10, and a record guard that forces the fallback loop.
    let fixture = fixture(35, 10, 4, true)?;
    let result = fixture
        .runtime
        .aggregate(
            &fixture.context,
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
    // Red is every third row of 35: indexes 0, 3, ..., 33.
    assert_eq!(result.groups[0].get("total"), Some(&Value::integer(12)));
    Ok(())
}

fn walk_traversal(fixture: &Fixture, sort: Vec<Sort>, limit: i32) -> Result<Vec<String>, Error> {
    let mut seen = Vec::new();
    let mut cursor: Option<Cursor> = None;
    loop {
        let page = fixture
            .runtime
            .traverse(
                &fixture.context,
                &name("members"),
                TraversalQuery {
                    source_pk: Key::string("o1"),
                    filter: None,
                    sort: sort.clone(),
                    limit: Some(limit),
                    cursor: cursor.take(),
                    fields: None,
                },
            )?
            .output;
        assert!(page.records.len() <= limit as usize);
        seen.extend(ids(&page.records));
        cursor = page.next_cursor;
        if cursor.is_none() {
            return Ok(seen);
        }
    }
}

#[test]
fn a_traversal_cursor_walk_visits_every_linked_row_once_under_ties() -> Result<(), Error> {
    // 900 members, 600 linked to o1: more than two 256-key chunks, and three
    // teams so the sort key ties across hundreds of rows.
    let fixture = fixture(900, 100, 4, false)?;
    let expected: BTreeSet<String> = (0..900)
        .filter(|index| index % 3 != 2)
        .map(|index| format!("m{index:04}"))
        .collect();
    for direction in [SortDirection::Asc, SortDirection::Desc] {
        let sort = vec![Sort {
            property: name("team"),
            direction,
        }];
        let seen = walk_traversal(&fixture, sort, 70)?;
        assert_eq!(seen.len(), expected.len());
        assert_eq!(seen.iter().cloned().collect::<BTreeSet<_>>(), expected);
    }
    let unsorted = walk_traversal(&fixture, Vec::new(), 100)?;
    let mut sorted = unsorted.clone();
    sorted.sort();
    assert_eq!(
        unsorted, sorted,
        "an unsorted traversal is in primary-key order"
    );
    Ok(())
}

#[test]
fn a_traversal_cursor_of_another_sort_is_rejected() -> Result<(), Error> {
    let fixture = fixture(30, 10, 4, false)?;
    let page = fixture
        .runtime
        .traverse(
            &fixture.context,
            &name("members"),
            TraversalQuery {
                source_pk: Key::string("o1"),
                filter: None,
                sort: Vec::new(),
                limit: Some(3),
                cursor: None,
                fields: None,
            },
        )?
        .output;
    let error = fixture
        .runtime
        .traverse(
            &fixture.context,
            &name("members"),
            TraversalQuery {
                source_pk: Key::string("o2"),
                filter: None,
                sort: Vec::new(),
                limit: Some(3),
                cursor: page.next_cursor,
                fields: None,
            },
        )
        .expect_err("another source");
    assert_eq!(error.code(), ErrorCode::Validation);
    Ok(())
}

#[test]
fn an_evicted_handle_reports_expiry() -> Result<(), Error> {
    let fixture = fixture(5, 10, 1, false)?;
    let InvocationOutput::Handle { handle, .. } =
        invoke(&fixture, "member.search", json!({}), true)?
    else {
        panic!("expected a handle");
    };
    let other = ExecutionContext::trusted(Principal::new("alice"), "turn-2")?;
    let call = invocation(&fixture.runtime, "member.search", json!({}), true)?;
    fixture.runtime.invoke(&other, call)?;
    let error = fixture
        .runtime
        .handle(&fixture.context, handle.id())
        .expect_err("evicted");
    assert_eq!(
        error.message(),
        "result handle h1 expired: its request was evicted; re-run the search and follow next_cursor"
    );
    Ok(())
}

#[test]
fn a_limit_above_the_maximum_is_rejected_not_clamped() -> Result<(), Error> {
    let fixture = fixture(30, 10, 4, false)?;
    let InvocationOutput::Handle { handle, .. } =
        invoke(&fixture, "member.search", json!({"limit": 10}), true)?
    else {
        panic!("expected a handle");
    };
    let expected = "limit must be between 1 and 10; follow next_cursor for more rows";
    let search = fixture.runtime.search(
        &fixture.context,
        &name("member"),
        Query {
            limit: Some(11),
            ..Query::default()
        },
    );
    assert_eq!(search.expect_err("above max").message(), expected);
    let zero = fixture.runtime.search(
        &fixture.context,
        &name("member"),
        Query {
            limit: Some(0),
            ..Query::default()
        },
    );
    assert_eq!(zero.expect_err("below min").message(), expected);
    let read = invoke(
        &fixture,
        "results.read",
        json!({"handle": handle.id(), "limit": 11}),
        false,
    );
    assert_eq!(read.expect_err("above max").message(), expected);
    let traverse = fixture.runtime.traverse(
        &fixture.context,
        &name("members"),
        TraversalQuery {
            source_pk: Key::string("o1"),
            filter: None,
            sort: Vec::new(),
            limit: Some(11),
            cursor: None,
            fields: None,
        },
    );
    assert_eq!(traverse.expect_err("above max").message(), expected);
    let empty = fixture.runtime.search(
        &fixture.context,
        &name("member"),
        Query {
            cursor: Some("".parse()?),
            ..Query::default()
        },
    );
    assert_eq!(
        empty.expect_err("empty cursor").code(),
        ErrorCode::Validation
    );
    Ok(())
}
