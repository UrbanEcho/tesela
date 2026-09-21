use super::*;

fn values(pairs: &[(&'static str, Value)]) -> BTreeMap<ApiName, Value> {
    pairs
        .iter()
        .map(|(name, value)| (ApiName::from(*name), value.clone()))
        .collect()
}

#[test]
fn a_complete_create_validates() {
    let ontology = sample_ontology();
    let customer = ontology.object_type("customer").expect("customer");
    let record = values(&[
        ("id", Value::string("c1")),
        ("name", Value::string("Acme")),
        ("revenue", Value::integer(10)),
        ("created_at", Value::string("2026-01-01T00:00:00Z")),
    ]);
    assert!(ontology.validate_create(customer, &record).is_ok());
}

#[test]
fn a_create_missing_a_required_property_is_rejected() {
    let ontology = sample_ontology();
    let customer = ontology.object_type("customer").expect("customer");
    let record = values(&[("id", Value::string("c1"))]);
    let error = ontology
        .validate_create(customer, &record)
        .expect_err("missing required properties must not validate");
    assert!(error.is(tesela_core::ErrorCode::Validation));
    assert_eq!(
        error.diagnostics().len(),
        3,
        "name, revenue and the trait's created_at are all required"
    );
}

#[test]
fn a_create_with_an_unknown_property_is_rejected() {
    let ontology = sample_ontology();
    let customer = ontology.object_type("customer").expect("customer");
    let record = values(&[
        ("id", Value::string("c1")),
        ("nmae", Value::string("typo")),
        ("name", Value::string("Acme")),
        ("revenue", Value::integer(10)),
        ("created_at", Value::string("2026-01-01T00:00:00Z")),
    ]);
    let error = ontology
        .validate_create(customer, &record)
        .expect_err("an unknown property must not validate");
    assert_eq!(error.diagnostics().len(), 1);
    assert!(error.diagnostics()[0].message.contains("nmae"));
}

#[test]
fn a_value_of_the_wrong_type_is_rejected() {
    let ontology = sample_ontology();
    let customer = ontology.object_type("customer").expect("customer");
    let record = values(&[
        ("id", Value::string("c1")),
        ("name", Value::string("Acme")),
        ("revenue", Value::string("ten")),
        ("created_at", Value::string("2026-01-01T00:00:00Z")),
    ]);
    let error = ontology
        .validate_create(customer, &record)
        .expect_err("a string is not an integer");
    assert!(error.diagnostics()[0].message.contains("expects Integer"));
}

#[test]
fn a_non_scalar_primary_key_value_is_rejected() {
    let ontology = sample_ontology();
    let customer = ontology.object_type("customer").expect("customer");
    let record = values(&[
        ("id", Value::integer(1)),
        ("name", Value::string("Acme")),
        ("created_at", Value::string("2026-01-01T00:00:00Z")),
    ]);
    // An integer id is a valid identity for a `String` property only if it
    // were a string; the shape check catches it first.
    assert!(ontology.validate_create(customer, &record).is_err());
}

#[test]
fn a_patch_validates_only_what_it_states() {
    let ontology = sample_ontology();
    let customer = ontology.object_type("customer").expect("customer");
    let patch = values(&[("name", Value::string("Acme"))]);
    assert!(ontology.validate_patch(customer, &patch).is_ok());

    let unknown = values(&[("nmae", Value::string("Acme"))]);
    assert!(ontology.validate_patch(customer, &unknown).is_err());
}

#[test]
fn a_patch_cannot_null_a_required_property() {
    let ontology = sample_ontology();
    let customer = ontology.object_type("customer").expect("customer");
    let patch = values(&[("name", Value::null())]);
    assert!(ontology.validate_patch(customer, &patch).is_err());
}

#[test]
fn allowed_values_gate_a_create() {
    let mut spec = sample_spec();
    spec.object_types[0].properties.push(Property {
        api_name: ApiName::from("status"),
        display: None,
        description: None,
        data_type: DataType::String,
        nullable: None,
        default: Some(Value::string("active")),
        allowed_values: Some(vec![Value::string("active"), Value::string("closed")]),
        tags: Vec::new(),
    });
    let ontology = CompiledOntology::compile(&spec).expect("compiles");
    let customer = ontology.object_type("customer").expect("customer");

    let good = values(&[
        ("id", Value::string("c1")),
        ("name", Value::string("Acme")),
        ("revenue", Value::integer(10)),
        ("created_at", Value::string("2026-01-01T00:00:00Z")),
        ("status", Value::string("closed")),
    ]);
    assert!(ontology.validate_create(customer, &good).is_ok());

    let bad = values(&[
        ("id", Value::string("c1")),
        ("name", Value::string("Acme")),
        ("revenue", Value::integer(10)),
        ("created_at", Value::string("2026-01-01T00:00:00Z")),
        ("status", Value::string("nonsense")),
    ]);
    let error = ontology
        .validate_create(customer, &bad)
        .expect_err("an undeclared enum value must not validate");
    assert!(error.diagnostics()[0].message.contains("allowed values"));
}

#[test]
fn lookups_by_name_and_id_agree() {
    let ontology = sample_ontology();
    for object_type in ontology.object_types() {
        assert_eq!(
            ontology
                .object_type(object_type.api_name())
                .map(|found| found.id()),
            Some(object_type.id())
        );
        assert_eq!(
            ontology
                .object_type_by_id(object_type.id())
                .map(|found| found.api_name()),
            Some(object_type.api_name())
        );
    }
    assert!(ontology.object_type("nope").is_none());
    assert_eq!(
        ontology
            .trait_by_name("auditable")
            .map(|found| found.properties().len()),
        Some(1)
    );
    assert_eq!(EntityKind::ALL.len(), 4);
}
