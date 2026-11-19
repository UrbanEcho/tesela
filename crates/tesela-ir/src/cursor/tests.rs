use super::*;
use std::collections::BTreeMap;

fn sorts() -> Vec<Sort> {
    vec![
        Sort {
            property: ApiName::from("a"),
            direction: SortDirection::Asc,
        },
        Sort {
            property: ApiName::from("b"),
            direction: SortDirection::Desc,
        },
    ]
}

fn record(pk: i64, a: Option<Value>, b: Option<Value>) -> Record {
    let mut values = BTreeMap::new();
    values.insert(ApiName::from("id"), Value::from(pk));
    if let Some(a) = a {
        values.insert(ApiName::from("a"), a);
    }
    if let Some(b) = b {
        values.insert(ApiName::from("b"), b);
    }
    Record::new(Key::I64(pk), values)
}

fn filter() -> Filter {
    Filter::eq(ApiName::from("tenant"), Value::string("acme"))
}

fn forge(token: &Token) -> Cursor {
    Cursor(hex::encode(serde_json::to_vec(token).unwrap()))
}

fn error_of(cursor: &Cursor, scope: &str, sort: &[Sort]) -> String {
    cursor
        .decode(scope, Some(&filter()), sort)
        .unwrap_err()
        .to_string()
}

#[test]
fn round_trips_values_nulls_and_missing() {
    let sort = sorts();
    let filter = filter();
    for (a, b) in [
        (Some(Value::from(3_i64)), Some(Value::string("x"))),
        (None, Some(Value::from(1_i64))),
        (Some(Value::null()), None),
    ] {
        let last = record(7, a.clone(), b.clone());
        let cursor = Cursor::encode("obj", Some(&filter), &sort, &last);
        let position = cursor.decode("obj", Some(&filter), &sort).unwrap();
        assert_eq!(position.primary_key, Key::I64(7));
        assert_eq!(position.values[0], a.filter(|value| !value.is_null()));
        assert_eq!(position.values[1], b.filter(|value| !value.is_null()));
        assert_eq!(position.compare(&last, &sort), Ordering::Equal);
        assert_eq!(cursor.to_string().parse::<Cursor>().unwrap(), cursor);
    }
}

#[test]
fn rejects_each_malformed_token_distinctly() {
    let sort = sorts();
    let good = Cursor::encode("obj", Some(&filter()), &sort, &record(1, None, None));
    let mut messages = vec![
        error_of(&Cursor(String::new()), "obj", &sort),
        error_of(&Cursor("zz".into()), "obj", &sort),
        error_of(&Cursor(hex::encode(b"not json")), "obj", &sort),
        error_of(&good, "other", &sort),
        error_of(
            &forge(&Token {
                v: VERSION,
                f: hex::encode(fingerprint("obj", Some(&filter()), &sort)),
                s: vec![vec![]],
                k: Key::I64(1),
            }),
            "obj",
            &sort,
        ),
    ];
    let bad_version = forge(&Token {
        v: 2,
        f: String::new(),
        s: vec![],
        k: Key::I64(1),
    });
    messages.push(error_of(&bad_version, "obj", &sort));
    assert!(messages[3].contains("different query"));
    assert!(messages[5].contains("version"));
    let mut unique = messages.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), messages.len(), "{messages:#?}");
}

#[test]
fn rejects_a_changed_filter() {
    let sort = sorts();
    let cursor = Cursor::encode("obj", Some(&filter()), &sort, &record(1, None, None));
    assert!(cursor.decode("obj", None, &sort).is_err());
}

#[test]
fn rejects_slots_with_more_than_one_value() {
    let sort = sorts();
    let token = Token {
        v: VERSION,
        f: hex::encode(fingerprint("obj", None, &sort)),
        s: vec![vec![Value::from(1_i64), Value::from(2_i64)], vec![]],
        k: Key::I64(1),
    };
    assert!(forge(&token).decode("obj", None, &sort).is_err());
}

#[test]
fn fingerprint_bytes_are_pinned() {
    let sort = vec![Sort {
        property: ApiName::from("a"),
        direction: SortDirection::Desc,
    }];
    let expected = serde_json::to_string(&serde_json::json!({
        "v": 1,
        "scope": "obj",
        "filter": filter(),
        "sort": sort,
    }))
    .unwrap();
    assert_eq!(
        expected,
        r#"{"filter":{"field":"tenant","op":"eq","value":"acme"},"scope":"obj","sort":[{"direction":"desc","property":"a"}],"v":1}"#
    );
    assert_eq!(
        hex::encode(fingerprint("obj", Some(&filter()), &sort)),
        "b2f87925daa12b99f6a69bf5f8f9859a"
    );
}

#[test]
fn json_maps_serialize_in_sorted_order() {
    // `preserve_order` would make map output depend on insertion order and
    // break fingerprint stability; it must stay disabled.
    let mut map = serde_json::Map::new();
    map.insert("b".into(), 1.into());
    map.insert("a".into(), 2.into());
    assert_eq!(
        serde_json::to_string(&serde_json::Value::Object(map)).unwrap(),
        r#"{"a":2,"b":1}"#
    );
}

/// Minimal evaluator for the operators `to_filter` emits, with the residual
/// engine's semantics for missing and null values.
fn matches(filter: &Filter, record: &Record) -> bool {
    let actual = |filter: &Filter| record.values.get(filter.field.as_ref().unwrap());
    match filter.op {
        FilterOp::And => filter.args.iter().all(|arg| matches(arg, record)),
        FilterOp::Or => filter.args.iter().any(|arg| matches(arg, record)),
        FilterOp::IsNull => actual(filter).is_none_or(Value::is_null),
        FilterOp::IsNotNull => actual(filter).is_some_and(|value| !value.is_null()),
        FilterOp::Eq => actual(filter).is_some_and(|value| Some(value) == filter.value.as_ref()),
        FilterOp::Gt => actual(filter).is_some_and(|value| Some(value) > filter.value.as_ref()),
        FilterOp::Lt => actual(filter).is_some_and(|value| Some(value) < filter.value.as_ref()),
        other => panic!("unexpected operator {other:?}"),
    }
}

#[test]
fn to_filter_matches_is_after_over_generated_records() {
    let slot_values = [
        None,
        Some(Value::null()),
        Some(Value::from(1_i64)),
        Some(Value::from(2_i64)),
    ];
    let mut records = Vec::new();
    let mut pk = 0;
    for a in &slot_values {
        for b in &slot_values {
            for _ in 0..2 {
                pk += 1;
                records.push(record(pk, a.clone(), b.clone()));
            }
        }
    }
    let pk_name = ApiName::from("id");
    for sort in [
        sorts(),
        vec![],
        vec![Sort {
            property: ApiName::from("b"),
            direction: SortDirection::Desc,
        }],
    ] {
        for last in &records {
            let cursor = Cursor::encode("obj", None, &sort, last);
            let position = cursor.decode("obj", None, &sort).unwrap();
            let predicate = position.to_filter(&sort, &pk_name);
            for candidate in &records {
                assert_eq!(
                    matches(&predicate, candidate),
                    position.is_after(candidate, &sort),
                    "sort {sort:?} last {last:?} candidate {candidate:?}"
                );
            }
        }
    }
}

#[test]
fn walking_by_position_visits_every_record_once() {
    let sort = sorts();
    let mut records: Vec<Record> = (1..=9)
        .map(|pk| {
            record(
                pk,
                Some(Value::from(pk % 3)),
                (pk % 2 == 0).then(|| Value::from(pk % 4)),
            )
        })
        .collect();
    records.sort_by(|l, r| crate::order::compare_records(l, r, &sort));
    let mut seen = Vec::new();
    let mut position: Option<KeysetPosition> = None;
    loop {
        let page: Vec<&Record> = records
            .iter()
            .filter(|row| position.as_ref().is_none_or(|p| p.is_after(row, &sort)))
            .take(2)
            .collect();
        let Some(last) = page.last() else { break };
        seen.extend(page.iter().map(|row| row.primary_key.clone()));
        position = Some(
            Cursor::encode("obj", None, &sort, last)
                .decode("obj", None, &sort)
                .unwrap(),
        );
    }
    let expected: Vec<Key> = records.iter().map(|row| row.primary_key.clone()).collect();
    assert_eq!(seen, expected);
}
