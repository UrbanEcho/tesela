use super::*;
use crate::{Aggregation, AggregationFunction};
use tesela_core::Principal;
use tesela_ir::Filter;

fn name(value: &str) -> ApiName {
    ApiName::new_unchecked(value)
}

fn context() -> ExecutionContext {
    ExecutionContext::trusted(Principal::new("test"), "test").expect("context")
}

fn order(id: i64, customer: &str, total: i64) -> (Key, BTreeMap<ApiName, Value>) {
    let values = BTreeMap::from([
        (name("id"), Value::integer(id)),
        (name("customer"), Value::string(customer)),
        (name("total"), Value::integer(total)),
    ]);
    (Key::I64(id), values)
}

fn seeded() -> (Arc<MemoryBackend>, ExecutionContext) {
    let backend = MemoryBackend::new();
    let context = context();
    for (id, customer, total) in [(1, "a", 10), (2, "b", 20), (3, "a", 30)] {
        let (key, values) = order(id, customer, total);
        backend
            .create(&context, &name("order"), key, values)
            .expect("create");
    }
    (backend, context)
}

fn linked(backend: &MemoryBackend, context: &ExecutionContext, customer: &str) -> Vec<Key> {
    backend
        .linked_keys(
            context,
            &name("orders"),
            &name("order"),
            &Key::string(customer),
        )
        .expect("linked")
}

#[test]
fn a_failed_batch_restores_rows_revisions_and_indexes() {
    let (backend, context) = seeded();
    backend.bind_link(name("orders"), name("customer"));
    assert_eq!(linked(&backend, &context, "a"), [Key::I64(1), Key::I64(3)]);
    let before = backend
        .get(&context, &name("order"), &Key::I64(1))
        .expect("get")
        .expect("row");
    let (_, created) = order(9, "a", 90);
    let batch = [
        Mutation::Create { values: created },
        Mutation::Update {
            primary_key: Key::I64(1),
            values: BTreeMap::from([(name("customer"), Value::string("c"))]),
            expected_revision: None,
            preconditions: Vec::new(),
        },
        Mutation::Delete {
            primary_key: Key::I64(3),
            expected_revision: None,
            preconditions: Vec::new(),
        },
        Mutation::Update {
            primary_key: Key::I64(404),
            values: BTreeMap::new(),
            expected_revision: None,
            preconditions: Vec::new(),
        },
    ];
    let error = backend
        .batch(&context, &name("order"), &name("id"), &batch)
        .expect_err("missing row aborts the batch");
    assert_eq!(error.code(), tesela_core::ErrorCode::NotFound);
    assert_eq!(backend.len(&name("order")).expect("len"), 3);
    let after = backend
        .get(&context, &name("order"), &Key::I64(1))
        .expect("get")
        .expect("row");
    assert_eq!(after, before);
    assert_eq!(linked(&backend, &context, "a"), [Key::I64(1), Key::I64(3)]);
    assert!(linked(&backend, &context, "c").is_empty());
}

#[test]
fn preconditions_see_earlier_changes_of_the_same_batch() {
    let (backend, context) = seeded();
    let precondition = Filter::eq(name("total"), Value::integer(11));
    let batch = [
        Mutation::Update {
            primary_key: Key::I64(1),
            values: BTreeMap::from([(name("total"), Value::integer(11))]),
            expected_revision: None,
            preconditions: Vec::new(),
        },
        Mutation::Update {
            primary_key: Key::I64(1),
            values: BTreeMap::from([(name("total"), Value::integer(12))]),
            expected_revision: None,
            preconditions: vec![precondition],
        },
    ];
    let result = backend
        .batch(&context, &name("order"), &name("id"), &batch)
        .expect("batch");
    assert_eq!(result.rows_affected, Some(2));
}

#[test]
fn the_link_index_follows_every_kind_of_write() {
    let (backend, context) = seeded();
    backend.bind_link(name("orders"), name("customer"));
    assert_eq!(linked(&backend, &context, "b"), [Key::I64(2)]);
    let object = name("order");
    backend
        .update(
            &context,
            &object,
            &Key::I64(2),
            BTreeMap::from([(name("customer"), Value::string("a"))]),
            None,
        )
        .expect("update");
    assert_eq!(
        linked(&backend, &context, "a"),
        [Key::I64(1), Key::I64(2), Key::I64(3)]
    );
    assert!(linked(&backend, &context, "b").is_empty());
    let (key, values) = order(4, "b", 1);
    backend
        .upsert(&context, &object, key, values, None)
        .expect("upsert");
    backend
        .delete(&context, &object, &Key::I64(1), None)
        .expect("delete");
    assert_eq!(linked(&backend, &context, "a"), [Key::I64(2), Key::I64(3)]);
    assert_eq!(linked(&backend, &context, "b"), [Key::I64(4)]);
    let unbound = backend.linked_keys(&context, &name("other"), &object, &Key::I64(1));
    assert!(unbound.is_err());
}

#[test]
fn aggregates_stream_the_table_without_materializing_rows() {
    let (backend, context) = seeded();
    let query = AggregateQuery {
        filter: Some(Filter::negate(Filter::eq(
            name("total"),
            Value::integer(10),
        ))),
        group_by: vec![name("customer")],
        aggregations: vec![Aggregation {
            function: AggregationFunction::Sum,
            property: Some(name("total")),
            alias: "sum".into(),
        }],
        ..AggregateQuery::default()
    };
    let groups = backend
        .aggregate(&context, &name("order"), &query)
        .expect("aggregate")
        .groups;
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[0]["customer"], Value::string("a"));
    assert_eq!(groups[0]["sum"], Value::float(30.0));
    assert_eq!(groups[1]["sum"], Value::float(20.0));
}

#[test]
fn rows_added_after_the_first_query_are_visible_to_it() {
    let (backend, context) = seeded();
    let object = name("order");
    let query = Query {
        filter: Some(Filter::eq(name("region"), Value::string("north"))),
        ..Query::default()
    };
    assert!(
        backend
            .search(&context, &object, &query)
            .expect("search")
            .records
            .is_empty()
    );
    backend
        .update(
            &context,
            &object,
            &Key::I64(2),
            BTreeMap::from([(name("region"), Value::string("north"))]),
            None,
        )
        .expect("update");
    let page = backend.search(&context, &object, &query).expect("search");
    assert_eq!(page.records.len(), 1);
    assert_eq!(page.records[0].primary_key, Key::I64(2));
    assert_eq!(
        page.records[0].values[&name("customer")],
        Value::string("b")
    );
}
