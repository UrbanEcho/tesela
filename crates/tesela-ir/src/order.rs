//! The one record ordering shared by queries, cursors and result handles.

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use tesela_core::{ApiName, Value};

use crate::Record;

/// A sort directive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sort {
    /// Property to sort by.
    pub property: ApiName,
    /// Sort direction.
    pub direction: SortDirection,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    /// Ascending order.
    Asc,
    /// Descending order.
    Desc,
}

fn present(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| !value.is_null())
}

/// Compare one sort slot, where an absent property and JSON `null` are the
/// same "missing" class.
///
/// Missing sorts before any present value under [`SortDirection::Asc`] and
/// after it under [`SortDirection::Desc`]. Present values use [`Value`]'s
/// total order. Record-vs-record and record-vs-cursor comparisons both go
/// through this function so they cannot disagree.
#[must_use]
#[inline]
pub fn compare_slot(
    left: Option<&Value>,
    right: Option<&Value>,
    direction: SortDirection,
) -> Ordering {
    let ordering = match (present(left), present(right)) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(left), Some(right)) => left.cmp(right),
    };
    match direction {
        SortDirection::Asc => ordering,
        SortDirection::Desc => ordering.reverse(),
    }
}

/// Compare records in the total order every Tesela page uses.
///
/// The order is the sort keys in sequence, where missing and JSON `null`
/// values sort before any present value under `Asc` and after it under
/// `Desc`, followed by the primary key ascending as a mandatory tiebreak. An
/// empty `sort` orders by primary key alone. The tiebreak makes pagination and
/// cross-backend merges deterministic.
#[must_use]
#[inline]
pub fn compare_records(left: &Record, right: &Record, sort: &[Sort]) -> Ordering {
    for item in sort {
        let ordering = compare_slot(
            left.values.get(&item.property),
            right.values.get(&item.property),
            item.direction,
        );
        if !ordering.is_eq() {
            return ordering;
        }
    }
    left.primary_key.cmp(&right.primary_key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use tesela_core::Key;

    fn record(pk: i64, score: Option<Value>) -> Record {
        let mut values = BTreeMap::new();
        if let Some(score) = score {
            values.insert(ApiName::from("score"), score);
        }
        Record::new(Key::I64(pk), values)
    }

    fn sort(direction: SortDirection) -> Vec<Sort> {
        vec![Sort {
            property: ApiName::from("score"),
            direction,
        }]
    }

    #[test]
    fn missing_and_null_are_one_class_before_present_under_asc() {
        let missing = record(1, None);
        let null = record(2, Some(Value::null()));
        let present = record(3, Some(Value::from(1_i64)));
        let asc = sort(SortDirection::Asc);
        assert_eq!(compare_records(&missing, &present, &asc), Ordering::Less);
        assert_eq!(compare_records(&null, &present, &asc), Ordering::Less);
        // Same class: the primary key decides.
        assert_eq!(compare_records(&missing, &null, &asc), Ordering::Less);
        assert_eq!(compare_records(&null, &missing, &asc), Ordering::Greater);
    }

    #[test]
    fn desc_puts_missing_last_and_keeps_pk_ascending() {
        let desc = sort(SortDirection::Desc);
        let missing = record(1, None);
        let low = record(2, Some(Value::from(1_i64)));
        let high = record(3, Some(Value::from(9_i64)));
        assert_eq!(compare_records(&high, &low, &desc), Ordering::Less);
        assert_eq!(compare_records(&low, &missing, &desc), Ordering::Less);
        let tie = record(4, Some(Value::from(9_i64)));
        assert_eq!(compare_records(&high, &tie, &desc), Ordering::Less);
    }

    #[test]
    fn unsorted_orders_by_primary_key() {
        assert_eq!(
            compare_records(&record(1, None), &record(2, None), &[]),
            Ordering::Less
        );
    }
}
