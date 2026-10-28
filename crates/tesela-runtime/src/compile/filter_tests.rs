use std::collections::BTreeMap;

use tesela_core::ErrorCode;

use crate::compile::tests::sample_object_type;

fn compile(filter: &Filter) -> Result<CompiledFilter, Error> {
    compile_filter(&sample_object_type(), filter)
}

#[test]
fn a_scalar_filter_resolves_its_field() {
    let filter = Filter::eq(ApiName::from("name"), Value::string("acme"));
    let compiled = compile(&filter).expect("filter compiles");
    let CompiledFilter::Eq(field, key) = compiled else {
        panic!("expected an equality filter");
    };
    assert_eq!(field.name().as_str(), "name");
    assert_eq!(key, Key::string("acme"));
}

#[test]
fn an_unknown_property_fails_closed() {
    let filter = Filter::eq(ApiName::from("nope"), Value::string("acme"));
    let error = compile(&filter).expect_err("unknown property must not compile");
    assert!(error.is(ErrorCode::UnknownProperty), "got {error:?}");
}

#[test]
fn a_fieldless_scalar_filter_fails_closed() {
    let filter = Filter {
        op: FilterOp::Eq,
        field: None,
        value: Some(Value::string("acme")),
        values: Vec::new(),
        args: Vec::new(),
    };
    let error = compile(&filter).expect_err("a fieldless filter must not compile");
    assert!(error.is(ErrorCode::Validation), "got {error:?}");
}

#[test]
fn a_valueless_scalar_filter_fails_closed() {
    let filter = Filter {
        op: FilterOp::Eq,
        field: Some(ApiName::from("name")),
        value: None,
        values: Vec::new(),
        args: Vec::new(),
    };
    assert!(compile(&filter).is_err());
}

#[test]
fn an_empty_logical_filter_fails_closed() {
    // An empty AND matches everything, so it must not compile.
    assert!(compile(&Filter::and(Vec::new())).is_err());
    assert!(compile(&Filter::or(Vec::new())).is_err());
}

#[test]
fn not_requires_exactly_one_operand() {
    assert!(
        compile(&Filter::negate(Filter::eq(
            ApiName::from("name"),
            Value::string("a")
        )))
        .is_ok()
    );
    let two = Filter {
        op: FilterOp::Not,
        field: None,
        value: None,
        values: Vec::new(),
        args: vec![
            Filter::eq(ApiName::from("name"), Value::string("a")),
            Filter::eq(ApiName::from("name"), Value::string("b")),
        ],
    };
    assert!(compile(&two).is_err());
}

#[test]
fn membership_compiles_to_a_hash_set() {
    let filter = Filter::is_in(
        ApiName::from("name"),
        vec![Value::string("a"), Value::string("b")],
    );
    let CompiledFilter::In(_, set) = compile(&filter).expect("filter compiles") else {
        panic!("expected an In filter");
    };
    assert_eq!(set.len(), 2);
    assert!(set.contains(&Key::string("a")));
}

#[test]
fn an_empty_membership_set_is_rejected() {
    let filter = Filter::is_in(ApiName::from("name"), Vec::new());
    assert!(compile(&filter).is_err());
}

#[test]
fn between_requires_two_values() {
    let one = Filter {
        op: FilterOp::Between,
        field: Some(ApiName::from("name")),
        value: None,
        values: vec![Value::string("a")],
        args: Vec::new(),
    };
    assert!(compile(&one).is_err());
    let two = Filter {
        op: FilterOp::Between,
        field: Some(ApiName::from("name")),
        value: None,
        values: vec![Value::string("a"), Value::string("z")],
        args: Vec::new(),
    };
    assert!(compile(&two).is_ok());
}

#[test]
fn comparing_a_scalar_property_to_a_container_is_rejected() {
    let filter = Filter::eq(ApiName::from("name"), Value::from(vec![Value::string("a")]));
    assert!(compile(&filter).is_err());
}

#[test]
fn like_requires_a_string_and_compiles_its_pattern() {
    let ok = Filter {
        op: FilterOp::Like,
        field: Some(ApiName::from("name")),
        value: Some(Value::string("a%")),
        values: Vec::new(),
        args: Vec::new(),
    };
    let CompiledFilter::Like(_, plan) = compile(&ok).expect("filter compiles") else {
        panic!("expected a Like filter");
    };
    assert!(plan.matches("abc"));
    assert!(!plan.matches("xabc"));

    let not_a_string = Filter {
        op: FilterOp::Like,
        field: Some(ApiName::from("name")),
        value: Some(Value::integer(1)),
        values: Vec::new(),
        args: Vec::new(),
    };
    assert!(compile(&not_a_string).is_err());
}

#[test]
fn suffix_matching_is_typed_and_allocation_free_at_evaluation() {
    let filter = Filter::ends_with(ApiName::from("name"), "example.test");
    let compiled = compile(&filter).expect("string suffix compiles");
    let CompiledFilter::EndsWith(_, suffix) = compiled else {
        panic!("expected an EndsWith filter");
    };
    assert_eq!(&*suffix, "example.test");
    assert!(compile(&Filter::ends_with(ApiName::from("revenue"), "x")).is_err());
}

#[test]
fn nested_logical_filters_compile_recursively() {
    let filter = Filter::and(vec![
        Filter::eq(ApiName::from("name"), Value::string("a")),
        Filter::or(vec![
            Filter::eq(ApiName::from("revenue"), Value::integer(1)),
            Filter::is_null(ApiName::from("name")),
        ]),
    ]);
    let compiled = compile(&filter).expect("filter compiles");
    let CompiledFilter::And(operands) = compiled else {
        panic!("expected an And filter");
    };
    assert_eq!(operands.len(), 2);
}

#[test]
fn fields_are_reported_once_each() {
    let filter = Filter::and(vec![
        Filter::eq(ApiName::from("name"), Value::string("a")),
        Filter::eq(ApiName::from("name"), Value::string("b")),
        Filter::eq(ApiName::from("revenue"), Value::integer(1)),
    ]);
    let compiled = compile(&filter).expect("filter compiles");
    let names: Vec<&str> = compiled
        .fields()
        .iter()
        .map(|field| field.name().as_str())
        .collect();
    assert_eq!(names, vec!["name", "revenue"]);
}

#[test]
fn a_field_reads_its_value_by_name() {
    let filter = Filter::eq(ApiName::from("name"), Value::string("acme"));
    let compiled = compile(&filter).expect("filter compiles");
    let field = compiled.fields()[0];
    let mut values = BTreeMap::new();
    values.insert(ApiName::from("name"), Value::string("acme"));
    assert_eq!(
        field.value_of(&values).and_then(Value::as_str),
        Some("acme")
    );
}
use super::*;

#[test]
fn evaluation_agrees_with_projecting_every_value_to_a_key() {
    let operands = [
        Value::string("beta"),
        Value::string(""),
        Value::integer(3),
        Value::float(2.5),
        Value::bool(true),
    ];
    let stored = [
        Some(Value::string("alpha")),
        Some(Value::string("beta")),
        Some(Value::integer(3)),
        Some(Value::integer(-9)),
        Some(Value::float(2.5)),
        Some(Value::bool(true)),
        Some(Value::null()),
        None,
    ];
    let members: Vec<_> = (0..12)
        .map(|index| Value::string(format!("m{index}")))
        .collect();
    let mut wide = members.clone();
    wide.push(Value::string("beta"));
    for field in ["name", "revenue"] {
        for operand in &operands {
            let key = Key::from(operand);
            let cases = [
                (FilterOp::Eq, Ordering::is_eq as fn(Ordering) -> bool),
                (FilterOp::Lt, Ordering::is_lt),
                (FilterOp::Gte, Ordering::is_ge),
            ];
            for (op, test) in cases {
                let filter = Filter {
                    op,
                    field: Some(ApiName::from(field)),
                    value: Some(operand.clone()),
                    values: Vec::new(),
                    args: Vec::new(),
                };
                let Ok(compiled) = compile(&filter) else {
                    continue;
                };
                for value in &stored {
                    let row: BTreeMap<_, _> = value
                        .iter()
                        .map(|value| (ApiName::from(field), value.clone()))
                        .collect();
                    let expected = value
                        .as_ref()
                        .is_some_and(|value| test(Key::from(value).cmp(&key)));
                    assert_eq!(compiled.matches(&row), expected, "{filter:?} on {value:?}");
                }
            }
        }
        for set in [
            wide.clone(),
            vec![Value::string("beta"), Value::string("zeta")],
        ] {
            let keys: Vec<_> = set.iter().map(Key::from).collect();
            let Ok(compiled) = compile(&Filter::is_in(ApiName::from(field), set.clone())) else {
                continue;
            };
            let negated = compile(&Filter::negate(Filter::is_in(ApiName::from(field), set))).ok();
            for value in &stored {
                let row: BTreeMap<_, _> = value
                    .iter()
                    .map(|value| (ApiName::from(field), value.clone()))
                    .collect();
                let expected = value
                    .as_ref()
                    .is_some_and(|value| keys.contains(&Key::from(value)));
                assert_eq!(compiled.matches(&row), expected, "In on {value:?}");
                if let Some(negated) = &negated {
                    assert_eq!(negated.matches(&row), !expected, "Not In on {value:?}");
                }
            }
        }
    }
}
