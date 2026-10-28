use std::collections::BTreeMap;

use tesela_core::{
    ApiName, DataType, ExecutionBudget, ExecutionContext, Key, Operation, Principal, RiskLevel,
    Value,
};
use tesela_ir::{ActionEffects, ObjectType, Property, Record, Spec};

use crate::{
    Capability, CapabilityAnnotations, CapabilityCatalog, DiffImpact, ResultHandle, ToolExposure,
    contains_phrase, semantic_diff, tokenize,
};

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

fn capability(name: &str, description: &str, exposure: ToolExposure) -> Capability {
    Capability {
        name: name.to_string(),
        description: description.to_string(),
        operation: Operation::Search,
        exposure,
        input_schema: Value::new(serde_json::json!({"type": "object"})),
        effects: ActionEffects::read_only(),
        annotations: CapabilityAnnotations {
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: false,
        },
        approval_required: false,
        risk: RiskLevel::Low,
        concurrency: None,
        requires_revision: false,
        preconditions: Vec::new(),
    }
}

fn catalog(capabilities: Vec<Capability>) -> CapabilityCatalog {
    CapabilityCatalog {
        generation: 1,
        fingerprint: Spec::default().fingerprint().expect("fingerprint"),
        capabilities,
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

#[test]
fn tokens_split_on_punctuation_and_lowercase() {
    assert_eq!(
        tokenize("Customer_Orders.search"),
        ["customer", "orders", "search"]
    );
    assert!(contains_phrase(
        &tokenize("list customer orders today"),
        &tokenize("customer_orders"),
    ));
    assert!(!contains_phrase(
        &tokenize("orders customer"),
        &tokenize("customer orders")
    ));
    assert!(!contains_phrase(&tokenize("anything"), &[]));
}

#[test]
fn search_matches_whole_tokens_not_substrings() {
    let catalog = catalog(vec![capability(
        "customer.search",
        "search customer objects",
        ToolExposure::Discoverable,
    )]);
    assert!(catalog.search("cust", 5).is_empty());
    assert_eq!(catalog.search("customer", 5)[0].name, "customer.search");
}

#[test]
fn search_ranks_exact_names_first_and_skips_hidden() {
    let catalog = catalog(vec![
        capability(
            "order.search",
            "search order objects",
            ToolExposure::Discoverable,
        ),
        capability(
            "customer.search",
            "search customer objects",
            ToolExposure::Discoverable,
        ),
        capability(
            "secret.search",
            "search secret objects",
            ToolExposure::Hidden,
        ),
    ]);
    let found = catalog.search("customer.search", 5);
    assert_eq!(found[0].name, "customer.search");
    assert!(catalog.search("secret", 5).is_empty());
}
