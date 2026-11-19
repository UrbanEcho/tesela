//! The pushdown predicate of a keyset position selects exactly the records the
//! position orders after, as evaluated by the real residual engine.

use std::collections::BTreeMap;
use tesela_backend::{Query, ScanOrder, Sort, SortDirection, execute_residual};
use tesela_core::{ApiName, Key, Value};
use tesela_ir::{Cursor, Record};

fn record(pk: i64, a: Option<Value>, b: Option<Value>) -> Record {
    let mut values = BTreeMap::new();
    values.insert(ApiName::from("id"), Value::from(pk));
    for (name, value) in [("a", a), ("b", b)] {
        if let Some(value) = value {
            values.insert(ApiName::from(name), value);
        }
    }
    Record::new(Key::I64(pk), values)
}

#[test]
fn to_filter_agrees_with_is_after_in_the_residual_engine() {
    let slots = [
        None,
        Some(Value::null()),
        Some(Value::from(1_i64)),
        Some(Value::from(2_i64)),
    ];
    let mut records = Vec::new();
    for a in &slots {
        for b in &slots {
            for _ in 0..2 {
                let pk = i64::try_from(records.len()).unwrap() + 1;
                records.push(record(pk, a.clone(), b.clone()));
            }
        }
    }
    let sort = vec![
        Sort {
            property: ApiName::from("a"),
            direction: SortDirection::Desc,
        },
        Sort {
            property: ApiName::from("b"),
            direction: SortDirection::Asc,
        },
    ];
    for last in &records {
        let position = Cursor::encode("obj", None, &sort, last)
            .decode("obj", None, &sort)
            .unwrap();
        let query = Query {
            filter: Some(position.to_filter(&sort, &ApiName::from("id"))),
            limit: Some(1000),
            ..Query::default()
        };
        let mut got: Vec<Key> = execute_residual(&records, &query, "obj", ScanOrder::Arbitrary)
            .unwrap()
            .records
            .into_iter()
            .map(|row| row.primary_key)
            .collect();
        got.sort();
        let want: Vec<Key> = records
            .iter()
            .filter(|row| position.is_after(row, &sort))
            .map(|row| row.primary_key.clone())
            .collect();
        assert_eq!(got, want, "last {last:?}");
    }
}
