use std::collections::BTreeMap;

use tesela_core::{ApiName, DataType, ExecutionBudget, ExecutionContext, Key, Principal, Value};
use tesela_ir::{ObjectType, Property, Record, Spec};

use super::*;

fn object(name: &'static str) -> ObjectType {
    ObjectType {
        api_name: ApiName::from(name),
        display: None,
        description: None,
        primary_key: ApiName::from("id"),
        properties: vec![Property {
            api_name: ApiName::from("id"),
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
    }
}

#[test]
fn semantic_diff_classifies_addition_and_removal() {
    let mut before = Spec::default();
    before.upsert_object_type(object("customer"));
    let mut after = Spec::default();
    after.upsert_object_type(object("invoice"));
    let changes = semantic_diff(&before, &after);
    assert!(changes.iter().any(|change| {
        change.path == "object_types.customer" && change.impact == DiffImpact::Breaking
    }));
    assert!(changes.iter().any(|change| {
        change.path == "object_types.invoice" && change.impact == DiffImpact::Safe
    }));
}

#[test]
fn result_handles_are_bounded_and_execution_scoped() {
    let context = ExecutionContext::new(
        Principal::new("agent"),
        Vec::new(),
        "request-a",
        None,
        tesela_core::CancellationToken::default(),
        ExecutionBudget {
            rows: Some(1),
            ..ExecutionBudget::default()
        },
    )
    .expect("context");
    let record = Record::new(
        Key::string("one"),
        BTreeMap::from([(ApiName::from("id"), Value::string("one"))]),
    );
    let handle = ResultHandle::new(&context, "result-1", vec![record], 1).expect("bounded");
    assert_eq!(handle.page(&context, 0, 1).expect("same context").len(), 1);
    let other =
        ExecutionContext::trusted(Principal::new("agent"), "request-b").expect("other context");
    assert!(handle.page(&other, 0, 1).is_err());
}
