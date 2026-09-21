//! What each macro expansion produces.
//!
//! The expansions refer to the hidden macro support module by absolute path,
//! so this test crate stands in for the facade without a dev-dependency cycle.

extern crate self as tesela;

pub mod __private {
    pub use serde_json;
    pub use tesela_core::*;
    pub use tesela_ir::*;
}

pub mod query {
    pub use tesela_core::Value;
    pub use tesela_ir::Filter;

    pub struct Field<T> {
        marker: std::marker::PhantomData<T>,
    }
    impl<T> Field<T> {
        pub const fn new(_: &'static str) -> Self {
            Self {
                marker: std::marker::PhantomData,
            }
        }
    }
}

use tesela_core::{DataType, LinkCardinality, RiskLevel};
use tesela_ir::{ActionEffects, Concurrency};

// ---------------------------------------------------------------------------
// #[derive(ObjectType)]
// ---------------------------------------------------------------------------

#[derive(tesela_macros::ObjectType)]
#[tesela(
    name = "customer",
    primary_key = "id",
    display = "Customer",
    description = "A buyer"
)]
struct Customer {
    #[tesela(description = "Primary key")]
    id: String,
    #[tesela(nullable)]
    email: String,
    revenue: f64,
}

#[test]
fn object_type_carries_its_semantics_and_nothing_else() {
    let object_type = Customer::tesela_object_type();
    let customer = Customer {
        id: "c1".to_string(),
        email: "c1@example.test".to_string(),
        revenue: 12.5,
    };
    assert_eq!(
        (
            customer.id.as_str(),
            customer.email.as_str(),
            customer.revenue
        ),
        ("c1", "c1@example.test", 12.5),
        "the derive leaves the struct as declared"
    );

    assert_eq!(object_type.api_name.as_ref(), "customer");
    assert_eq!(object_type.display.as_deref(), Some("Customer"));
    assert_eq!(object_type.description.as_deref(), Some("A buyer"));
    assert_eq!(object_type.primary_key.as_ref(), "id");
    assert!(object_type.traits.is_empty());

    let properties: Vec<_> = object_type
        .properties
        .iter()
        .map(|property| property.api_name.as_ref())
        .collect();
    assert_eq!(
        properties,
        ["id", "email", "revenue"],
        "properties keep declaration order"
    );

    let id = &object_type.properties[0];
    assert_eq!(id.data_type, DataType::String);
    assert_eq!(id.nullable, None, "a required field declares nothing");
    assert_eq!(id.description.as_deref(), Some("Primary key"));

    let email = &object_type.properties[1];
    assert_eq!(email.nullable, Some(true));

    let revenue = &object_type.properties[2];
    assert_eq!(revenue.data_type, DataType::Float);
    assert_eq!(revenue.nullable, None);
}

// ---------------------------------------------------------------------------
// #[derive(LinkType)]
// ---------------------------------------------------------------------------

#[derive(tesela_macros::LinkType)]
#[tesela(
    from = "customer",
    to = "order",
    cardinality = "one_to_many",
    inverse = "placed_by",
    display = "Orders"
)]
struct CustomerOrders;

#[test]
fn link_type_is_directional_and_has_no_storage_mapping() {
    let link = CustomerOrders::tesela_link_type();

    assert_eq!(link.api_name.as_ref(), "customer_orders");
    assert_eq!(link.from.as_ref(), "customer");
    assert_eq!(link.to.as_ref(), "order");
    assert_eq!(link.cardinality, LinkCardinality::OneToMany);
    assert_eq!(
        link.inverse.as_ref().map(|name| name.as_ref()),
        Some("placed_by")
    );
    assert_eq!(link.display.as_deref(), Some("Orders"));
}

#[derive(tesela_macros::LinkType)]
#[tesela(from = "customer", to = "order")]
struct PlacedBy;

#[test]
fn link_type_defaults_to_one_to_many_without_an_inverse() {
    let link = PlacedBy::tesela_link_type();

    assert_eq!(link.api_name.as_ref(), "placed_by");
    assert_eq!(link.cardinality, LinkCardinality::OneToMany);
    assert_eq!(link.inverse, None);
    assert_eq!(link.display, None);
}

// ---------------------------------------------------------------------------
// #[derive(TraitDef)]
// ---------------------------------------------------------------------------

#[derive(tesela_macros::TraitDef)]
#[tesela(name = "auditable", description = "Carries an audit trail")]
struct Auditable {
    created_at: String,
    #[tesela(nullable, description = "Who changed it last")]
    updated_by: String,
}

#[test]
fn trait_def_contributes_properties_without_a_primary_key() {
    let trait_def = Auditable::tesela_trait();
    let auditable = Auditable {
        created_at: "2026-01-01T00:00:00Z".to_string(),
        updated_by: "miguel".to_string(),
    };
    assert_eq!(
        (auditable.created_at.as_str(), auditable.updated_by.as_str()),
        ("2026-01-01T00:00:00Z", "miguel")
    );

    assert_eq!(trait_def.api_name.as_ref(), "auditable");
    assert_eq!(
        trait_def.description.as_deref(),
        Some("Carries an audit trail")
    );
    assert_eq!(trait_def.properties.len(), 2);

    let created_at = &trait_def.properties[0];
    assert_eq!(created_at.api_name.as_ref(), "created_at");
    assert_eq!(created_at.nullable, None);

    let updated_by = &trait_def.properties[1];
    assert_eq!(updated_by.nullable, Some(true));
    assert_eq!(
        updated_by.description.as_deref(),
        Some("Who changed it last")
    );
}

// ---------------------------------------------------------------------------
// #[action]
// ---------------------------------------------------------------------------

#[tesela_macros::action(
    subject = "customer",
    risk = "high",
    effects = "external",
    idempotent,
    concurrency = "optimistic",
    display = "Sync a customer",
    description = "Pushes the record to the billing system"
)]
fn sync_customer(id: String, force: bool) -> String {
    format!("{id}:{force}")
}

#[test]
fn an_action_declares_effects_risk_and_concurrency() {
    let action = SyncCustomerAction::tesela_action_type();

    assert_eq!(action.api_name.as_ref(), "sync_customer");
    assert_eq!(
        action.subject.as_ref().map(|name| name.as_ref()),
        Some("customer")
    );
    assert_eq!(action.risk_level, RiskLevel::High);
    assert_eq!(
        action.effects,
        ActionEffects {
            reads: true,
            writes: true,
            external: true,
            idempotent: true,
        }
    );
    assert_eq!(action.concurrency, Concurrency::Optimistic);
    assert_eq!(action.display.as_deref(), Some("Sync a customer"));
    assert_eq!(
        action.description.as_deref(),
        Some("Pushes the record to the billing system")
    );

    // The annotated function stays an ordinary function.
    assert_eq!(sync_customer("c1".to_string(), true), "c1:true");
}

#[test]
fn an_action_schema_is_inferred_from_its_parameters() {
    let schema = SyncCustomerAction::tesela_action_type()
        .input_schema
        .expect("parameters produce an input schema");

    assert_eq!(schema["type"], "object");
    assert_eq!(schema["properties"]["id"]["type"], "string");
    assert_eq!(schema["properties"]["force"]["type"], "boolean");
}

#[tesela_macros::action]
fn touch_nothing() {}

#[test]
fn an_undeclared_action_is_assumed_to_write() {
    let action = TouchNothingAction::tesela_action_type();
    touch_nothing();

    assert_eq!(action.api_name.as_ref(), "touch_nothing");
    assert_eq!(action.subject, None);
    assert_eq!(action.risk_level, RiskLevel::Low);
    assert_eq!(action.display.as_deref(), Some("touch_nothing"));
    assert_eq!(action.description, None);
    assert_eq!(
        action.effects,
        ActionEffects::default(),
        "an action that declares nothing is assumed to write and is not idempotent"
    );
    assert_eq!(action.concurrency, Concurrency::Serial);
    assert_eq!(
        action.input_schema, None,
        "a parameterless action declares no input schema"
    );
}
