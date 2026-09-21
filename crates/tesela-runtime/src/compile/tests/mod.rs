use super::*;
use tesela_core::{DataType, LinkCardinality, RiskLevel};
use tesela_ir::{ActionType, Concurrency, EntityKind, Property, Trait};

fn property(name: &'static str, data_type: DataType) -> Property {
    Property {
        api_name: ApiName::from(name),
        display: None,
        description: None,
        data_type,
        nullable: None,
        default: None,
        allowed_values: None,
        tags: Vec::new(),
    }
}

pub(crate) fn sample_object_type() -> CompiledObjectType {
    sample_ontology()
        .object_type("customer")
        .expect("customer compiles")
        .clone()
}

/// A small but representative ontology: a trait, two object types, a link
/// with an inverse, and an action with a subject.
pub(crate) fn sample_spec() -> Spec {
    let mut spec = Spec::default();
    spec.upsert_trait(Trait {
        api_name: ApiName::from("auditable"),
        display: None,
        description: None,
        properties: vec![property("created_at", DataType::Timestamp)],
    });
    spec.upsert_object_type(ObjectType {
        api_name: ApiName::from("customer"),
        display: Some("Customer".to_string()),
        description: None,
        primary_key: ApiName::from("id"),
        properties: vec![
            property("id", DataType::String),
            property("name", DataType::String),
            property("revenue", DataType::Integer),
        ],
        traits: vec![ApiName::from("auditable")],
        tags: Vec::new(),
        deprecated_at: None,
    });
    spec.upsert_object_type(ObjectType {
        api_name: ApiName::from("order"),
        display: None,
        description: None,
        primary_key: ApiName::from("id"),
        properties: vec![property("id", DataType::String)],
        traits: Vec::new(),
        tags: Vec::new(),
        deprecated_at: None,
    });
    spec.upsert_link_type(LinkType {
        api_name: ApiName::from("owns"),
        display: None,
        from: ApiName::from("customer"),
        to: ApiName::from("order"),
        cardinality: LinkCardinality::OneToMany,
        inverse: Some(ApiName::from("owned_by")),
        deprecated_at: None,
    });
    spec.upsert_link_type(LinkType {
        api_name: ApiName::from("owned_by"),
        display: None,
        from: ApiName::from("order"),
        to: ApiName::from("customer"),
        cardinality: LinkCardinality::ManyToMany,
        inverse: Some(ApiName::from("owns")),
        deprecated_at: None,
    });
    spec.upsert_action(ActionType {
        api_name: ApiName::from("archive_customer"),
        display: None,
        description: None,
        subject: Some(ApiName::from("customer")),
        input_schema: None,
        output_schema: None,
        risk_level: RiskLevel::Medium,
        effects: ActionEffects::new(),
        reads: Vec::new(),
        writes: Vec::new(),
        creates: Vec::new(),
        external_effects: Vec::new(),
        approval: ApprovalPolicy::Policy,
        requires_revision: false,
        preconditions: Vec::new(),
        concurrency: Concurrency::Serial,
        idempotency_key: None,
        deprecated_at: None,
    });
    spec
}

pub(crate) fn sample_ontology() -> CompiledOntology {
    CompiledOntology::compile(&sample_spec()).expect("the sample spec compiles")
}

fn diagnostics_of(spec: &Spec) -> Vec<String> {
    CompiledOntology::compile(spec)
        .expect_err("spec should not compile")
        .into_diagnostics()
        .into_iter()
        .filter(Diagnostic::is_error)
        .map(|diagnostic| diagnostic.message)
        .collect()
}

fn assert_reports(spec: &Spec, fragment: &str) {
    let messages = diagnostics_of(spec);
    assert!(
        messages.iter().any(|message| message.contains(fragment)),
        "expected a diagnostic containing {fragment:?}, got {messages:#?}"
    );
}

#[test]
fn a_representative_spec_compiles() {
    let ontology = sample_ontology();
    assert_eq!(ontology.object_types().len(), 2);
    assert_eq!(
        ontology.object_type("customer").expect("customer").id(),
        ObjectTypeId::new(0)
    );
    assert_eq!(
        ontology.object_type("order").expect("order").id(),
        ObjectTypeId::new(1)
    );
    assert_eq!(
        ontology.link_type("owns").expect("owns").id(),
        LinkId::new(0)
    );
    assert_eq!(
        ontology.action("archive_customer").expect("action").id(),
        ActionId::new(0)
    );
}

#[test]
fn traits_are_flattened_into_the_object_type() {
    let customer = sample_ontology();
    let customer = customer.object_type("customer").expect("customer");
    // Declared properties keep their order, then the trait's.
    let names: Vec<&str> = customer
        .properties()
        .iter()
        .map(|property| property.api_name().as_str())
        .collect();
    assert_eq!(names, vec!["id", "name", "revenue", "created_at"]);
    let inherited = customer.property("created_at").expect("inherited property");
    assert_eq!(
        inherited.origin(),
        &PropertyOrigin::Trait(ApiName::from("auditable"))
    );
    assert_eq!(
        customer.property("id").expect("declared").origin(),
        &PropertyOrigin::Declared
    );
}

#[test]
fn property_ids_are_dense_and_type_local() {
    let ontology = sample_ontology();
    let customer = ontology.object_type("customer").expect("customer");
    for (index, property) in customer.properties().iter().enumerate() {
        assert_eq!(property.id(), PropertyId::new(index as u32));
        assert_eq!(
            customer
                .property_by_id(property.id())
                .map(|found| found.api_name()),
            Some(property.api_name())
        );
    }
    assert_eq!(customer.primary_key_id(), PropertyId::new(0));
    assert_eq!(customer.primary_key().api_name().as_str(), "id");
}

#[test]
fn a_duplicate_object_type_name_is_rejected() {
    let mut spec = sample_spec();
    spec.object_types.push(spec.object_types[0].clone());
    assert_reports(&spec, "declared more than once");
}

#[test]
fn an_unsupported_spec_version_is_rejected() {
    let mut spec = sample_spec();
    spec.version = tesela_core::Version::new("tesela.spec.v2");
    assert_reports(&spec, "unsupported spec version");
}

#[test]
fn a_primary_key_that_is_not_a_property_is_rejected() {
    let mut spec = sample_spec();
    spec.object_types[0].primary_key = ApiName::from("nope");
    assert_reports(&spec, "which is not one of its properties");
}

#[test]
fn a_non_scalar_primary_key_is_rejected() {
    let mut spec = sample_spec();
    spec.object_types[0].properties[0].data_type = DataType::Json;
    assert_reports(&spec, "an identity must be a scalar");
}

#[test]
fn a_nullable_primary_key_is_rejected() {
    let mut spec = sample_spec();
    spec.object_types[0].properties[0].nullable = Some(true);
    assert_reports(&spec, "a record's identity cannot be absent");
}

#[test]
fn a_property_colliding_with_a_trait_property_is_rejected() {
    let mut spec = sample_spec();
    spec.object_types[0]
        .properties
        .push(property("created_at", DataType::String));
    assert_reports(&spec, "is declared more than once");
}

#[test]
fn an_unknown_trait_is_rejected() {
    let mut spec = sample_spec();
    spec.object_types[0].traits.push(ApiName::from("missing"));
    assert_reports(&spec, "which the spec does not define");
}

#[test]
fn a_link_endpoint_that_does_not_resolve_is_rejected() {
    let mut spec = sample_spec();
    spec.link_types[0].to = ApiName::from("invoice");
    assert_reports(&spec, "which the spec does not define");
}

#[test]
fn an_asymmetric_inverse_is_rejected_exactly_once() {
    let mut spec = sample_spec();
    spec.link_types[1].inverse = None;
    let messages = diagnostics_of(&spec);
    let asymmetric = messages
        .iter()
        .filter(|message| message.contains("does not declare"))
        .count();
    assert_eq!(asymmetric, 1, "got {messages:#?}");
}

#[test]
fn an_inverse_between_the_wrong_endpoints_is_rejected() {
    let mut spec = sample_spec();
    spec.link_types[1].to = ApiName::from("order");
    assert_reports(&spec, "must run the other way");
}

#[test]
fn an_unknown_link_inverse_is_rejected() {
    let mut spec = sample_spec();
    spec.link_types[0].inverse = Some(ApiName::from("missing"));
    assert_reports(&spec, "which the spec does not define");
}

#[test]
fn an_action_subject_that_does_not_resolve_is_rejected() {
    let mut spec = sample_spec();
    spec.actions[0].subject = Some(ApiName::from("invoice"));
    assert_reports(&spec, "which the spec does not define");
}

#[test]
fn an_enum_property_without_allowed_values_is_rejected() {
    let mut spec = sample_spec();
    spec.object_types[0]
        .properties
        .push(property("status", DataType::Enum));
    assert_reports(&spec, "without allowed_values");
}

#[test]
fn a_default_outside_allowed_values_is_rejected() {
    let mut spec = sample_spec();
    let status = property("status", DataType::String);
    spec.object_types[0].properties.push(Property {
        allowed_values: Some(vec![Value::string("active")]),
        default: Some(Value::string("closed")),
        ..status
    });
    assert_reports(&spec, "which its allowed_values do not include");
}

#[test]
fn a_required_property_defaulting_to_null_is_rejected() {
    let mut spec = sample_spec();
    spec.object_types[0].properties[1].default = Some(Value::null());
    assert_reports(&spec, "defaulting to null");
}

#[test]
fn a_zero_dimension_vector_is_rejected() {
    let mut spec = sample_spec();
    spec.object_types[0]
        .properties
        .push(property("embedding", DataType::Vector(0)));
    assert_reports(&spec, "with dimension 0");
}

#[test]
fn every_problem_is_reported_at_once() {
    let mut spec = sample_spec();
    spec.object_types[0].primary_key = ApiName::from("nope");
    // Drop the inverse declarations first: a broken link endpoint also
    // trips the inverse checks, and this test counts independent faults,
    // not the cascade one fault produces.
    spec.link_types[0].inverse = None;
    spec.link_types[1].inverse = None;
    spec.link_types[0].to = ApiName::from("invoice");
    spec.actions[0].subject = Some(ApiName::from("invoice"));
    let error = CompiledOntology::compile(&spec).expect_err("spec should not compile");
    let codes: Vec<&DiagnosticCode> = error.errors().map(|item| &item.code).collect();
    assert_eq!(codes.len(), 3, "got {codes:#?}");
    let messages: Vec<&str> = error.errors().map(|item| item.message.as_str()).collect();
    assert!(messages.iter().any(|item| item.contains("'nope'")));
    assert_eq!(
        messages
            .iter()
            .filter(|item| item.contains("'invoice'"))
            .count(),
        2,
        "the broken link endpoint and the broken action subject are separate faults"
    );
}

#[test]
fn warnings_do_not_block_compilation() {
    let mut spec = sample_spec();
    spec.actions[0].risk_level = RiskLevel::High;
    spec.actions[0].effects = ActionEffects::read_only();
    let ontology = CompiledOntology::compile(&spec).expect("warnings are not errors");
    assert_eq!(ontology.generation(), 0);
    assert!(
        ontology
            .action("archive_customer")
            .expect("action")
            .is_read_only()
    );
}

#[test]
fn a_spec_error_carries_its_diagnostics_into_the_error_type() {
    let mut spec = sample_spec();
    spec.object_types[0].primary_key = ApiName::from("nope");
    let error: Error = CompiledOntology::compile(&spec)
        .expect_err("spec should not compile")
        .into();
    assert!(error.is(tesela_core::ErrorCode::Validation));
    assert_eq!(error.diagnostics().len(), 1);
}

#[test]
fn generation_is_stamped_not_compiled() {
    let ontology = sample_ontology().with_generation(7);
    assert_eq!(ontology.generation(), 7);
    assert_eq!(ontology.fingerprint(), sample_ontology().fingerprint());
}

#[test]
fn the_fingerprint_identifies_the_spec() {
    let mut other = sample_spec();
    other.traits.clear();
    other.object_types[0].traits.clear();
    assert_ne!(
        sample_ontology().fingerprint(),
        CompiledOntology::compile(&other)
            .expect("compiles")
            .fingerprint()
    );
}

#[test]
fn defaults_fill_only_gaps() {
    let mut spec = sample_spec();
    spec.object_types[0].properties.push(Property {
        api_name: ApiName::from("status"),
        display: None,
        description: None,
        data_type: DataType::String,
        nullable: Some(true),
        default: Some(Value::string("active")),
        allowed_values: None,
        tags: Vec::new(),
    });
    let ontology = CompiledOntology::compile(&spec).expect("compiles");
    let customer = ontology.object_type("customer").expect("customer");

    let mut values = BTreeMap::new();
    values.insert(ApiName::from("id"), Value::string("c1"));
    customer.apply_defaults(&mut values);
    assert_eq!(values.get("status").and_then(Value::as_str), Some("active"));

    let mut explicit = BTreeMap::new();
    explicit.insert(ApiName::from("id"), Value::string("c1"));
    explicit.insert(ApiName::from("status"), Value::null());
    customer.apply_defaults(&mut explicit);
    assert!(
        explicit.get("status").expect("present").is_null(),
        "an explicit null is a value, not a gap"
    );
}

mod validation_tests;
