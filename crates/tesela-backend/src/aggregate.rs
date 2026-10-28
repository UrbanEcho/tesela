//! One-pass grouped aggregation shared by every residual path.
//!
//! Groups are found by hashing the grouped values in place, so a row that
//! joins an existing group allocates nothing. Only the group state and the
//! running extrema are retained, never the rows.

use std::collections::{BTreeMap, HashMap};

use tesela_core::{ApiName, Key, Value, hash_values};
use tesela_ir::{AggregateResult, Record};

use crate::residual::RowAccess;
use crate::{AggregateQuery, Aggregation, AggregationFunction};

const NONE: u32 = u32::MAX;

#[derive(Default, Clone)]
struct State {
    count: u64,
    numeric_count: u64,
    sum: f64,
    min: Option<Value>,
    max: Option<Value>,
}

impl State {
    fn add(&mut self, value: Option<&Value>) {
        self.count += 1;
        let Some(value) = value else { return };
        if let Some(number) = value.as_f64() {
            self.sum += number;
            self.numeric_count += 1;
        }
        if self.min.as_ref().is_none_or(|current| value < current) {
            self.min = Some(value.clone());
        }
        if self.max.as_ref().is_none_or(|current| value > current) {
            self.max = Some(value.clone());
        }
    }

    fn finish(self, function: AggregationFunction) -> Value {
        match function {
            AggregationFunction::Count => Value::integer(self.count as i64),
            AggregationFunction::Sum => Value::float(self.sum),
            AggregationFunction::Avg if self.numeric_count > 0 => {
                Value::float(self.sum / self.numeric_count as f64)
            }
            AggregationFunction::Avg => Value::null(),
            AggregationFunction::Min => self.min.unwrap_or_default(),
            AggregationFunction::Max => self.max.unwrap_or_default(),
        }
    }
}

struct Group {
    key: Vec<Key>,
    states: Vec<State>,
    next: u32,
}

/// Grouped aggregation over rows read through a resolved field layout.
pub(crate) struct Aggregator<'q, S> {
    fields: Vec<ApiName>,
    group_slots: Vec<S>,
    aggregations: &'q [Aggregation],
    value_slots: Vec<Option<S>>,
    groups: Vec<Group>,
    heads: HashMap<u64, u32>,
}

impl<'q, S: Clone> Aggregator<'q, S> {
    pub(crate) fn new<A: RowAccess<Slot = S>>(
        access: &A,
        fields: Vec<ApiName>,
        aggregations: &'q [Aggregation],
    ) -> Self {
        Self {
            group_slots: fields.iter().map(|field| access.slot(field)).collect(),
            value_slots: aggregations
                .iter()
                .map(|item| item.property.as_ref().map(|field| access.slot(field)))
                .collect(),
            fields,
            aggregations,
            groups: Vec::new(),
            heads: HashMap::new(),
        }
    }

    pub(crate) fn add<A: RowAccess<Slot = S>>(&mut self, access: &A, row: &A::Row) {
        let hash = hash_values(self.group_slots.iter().map(|slot| access.value(row, slot)));
        let index = self.group_of(access, row, hash);
        let group = &mut self.groups[index];
        for (state, slot) in group.states.iter_mut().zip(&self.value_slots) {
            state.add(slot.as_ref().and_then(|slot| access.value(row, slot)));
        }
    }

    fn group_of<A: RowAccess<Slot = S>>(&mut self, access: &A, row: &A::Row, hash: u64) -> usize {
        let mut cursor = self.heads.get(&hash).copied().unwrap_or(NONE);
        while cursor != NONE {
            let group = &self.groups[cursor as usize];
            let same = group.key.iter().zip(&self.group_slots).all(|(key, slot)| {
                match access.value(row, slot) {
                    Some(value) => key.eq_value(value),
                    None => key.is_null(),
                }
            });
            if same {
                return cursor as usize;
            }
            cursor = group.next;
        }
        let index = self.groups.len();
        let key = self
            .group_slots
            .iter()
            .map(|slot| access.value(row, slot).map_or(Key::Null, Key::from))
            .collect();
        let next = self.heads.insert(hash, index as u32).unwrap_or(NONE);
        self.groups.push(Group {
            key,
            states: vec![State::default(); self.aggregations.len()],
            next,
        });
        index
    }

    pub(crate) fn finish(mut self) -> AggregateResult {
        if self.groups.is_empty() && self.fields.is_empty() {
            self.groups.push(Group {
                key: Vec::new(),
                states: vec![State::default(); self.aggregations.len()],
                next: NONE,
            });
        }
        self.groups.sort_by(|left, right| left.key.cmp(&right.key));
        AggregateResult {
            groups: self
                .groups
                .into_iter()
                .map(|group| {
                    let mut row = BTreeMap::new();
                    for (field, key) in self.fields.iter().zip(group.key) {
                        row.insert(field.to_string(), key.to_value());
                    }
                    for (aggregation, state) in self.aggregations.iter().zip(group.states) {
                        row.insert(
                            aggregation.alias.clone(),
                            state.finish(aggregation.function),
                        );
                    }
                    row
                })
                .collect(),
        }
    }
}

/// Streaming grouped aggregation over [`Record`]s.
///
/// Hosts without native aggregation feed pages through [`extend`] and read the
/// groups once at the end; the accumulator keeps group state, not rows.
///
/// [`extend`]: AggregateAccumulator::extend
pub struct AggregateAccumulator<'q>(Aggregator<'q, ApiName>);

impl<'q> AggregateAccumulator<'q> {
    /// Start an aggregation. Grouped fields named in `redactions` are dropped
    /// from the output groups.
    #[must_use]
    pub fn new(query: &'q AggregateQuery, redactions: &[ApiName]) -> Self {
        let visible = query
            .group_by
            .iter()
            .filter(|field| !redactions.contains(field))
            .cloned()
            .collect();
        Self(Aggregator::new(
            &crate::residual::Borrowed::default(),
            visible,
            &query.aggregations,
        ))
    }

    /// Fold one batch of records into the groups.
    pub fn extend(&mut self, records: &[Record]) {
        let access = crate::residual::Borrowed::default();
        for record in records {
            self.0.add(&access, &record);
        }
    }

    /// Finish the aggregation, ordering groups by their key.
    #[must_use]
    pub fn finish(self) -> AggregateResult {
        self.0.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: i64, region: Option<&str>, total: Option<i64>) -> Record {
        let mut values = BTreeMap::new();
        if let Some(region) = region {
            values.insert(ApiName::new_unchecked("region"), Value::string(region));
        }
        if let Some(total) = total {
            values.insert(ApiName::new_unchecked("total"), Value::integer(total));
        }
        Record::new(Key::I64(id), values)
    }

    fn query(group_by: &[&str]) -> AggregateQuery {
        AggregateQuery {
            filter: None,
            group_by: group_by.iter().map(ApiName::new_unchecked).collect(),
            aggregations: vec![
                Aggregation {
                    function: AggregationFunction::Count,
                    property: None,
                    alias: "n".into(),
                },
                Aggregation {
                    function: AggregationFunction::Sum,
                    property: Some(ApiName::new_unchecked("total")),
                    alias: "sum".into(),
                },
                Aggregation {
                    function: AggregationFunction::Max,
                    property: Some(ApiName::new_unchecked("total")),
                    alias: "max".into(),
                },
            ],
            ..AggregateQuery::default()
        }
    }

    #[test]
    fn groups_are_ordered_and_missing_fields_group_as_null() {
        let query = query(&["region"]);
        let mut accumulator = AggregateAccumulator::new(&query, &[]);
        accumulator.extend(&[
            record(1, Some("south"), Some(5)),
            record(2, None, Some(1)),
            record(3, Some("north"), None),
            record(4, Some("south"), Some(7)),
        ]);
        let groups = accumulator.finish().groups;
        let regions: Vec<_> = groups.iter().map(|group| group["region"].clone()).collect();
        assert_eq!(
            regions,
            [
                Value::null(),
                Value::string("north"),
                Value::string("south")
            ]
        );
        assert_eq!(groups[2]["n"], Value::integer(2));
        assert_eq!(groups[2]["sum"], Value::float(12.0));
        assert_eq!(groups[2]["max"], Value::integer(7));
        assert_eq!(groups[1]["max"], Value::null());
    }

    #[test]
    fn an_empty_ungrouped_aggregation_still_yields_one_row() {
        let query = query(&[]);
        let groups = AggregateAccumulator::new(&query, &[]).finish().groups;
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["n"], Value::integer(0));
        let query = self::query(&["region"]);
        assert!(
            AggregateAccumulator::new(&query, &[])
                .finish()
                .groups
                .is_empty()
        );
    }

    #[test]
    fn redacted_group_fields_collapse_into_one_group() {
        let query = query(&["region"]);
        let mut accumulator =
            AggregateAccumulator::new(&query, &[ApiName::new_unchecked("region")]);
        accumulator.extend(&[record(1, Some("a"), Some(1)), record(2, Some("b"), Some(2))]);
        let groups = accumulator.finish().groups;
        assert_eq!(groups.len(), 1);
        assert!(!groups[0].contains_key("region"));
        assert_eq!(groups[0]["n"], Value::integer(2));
    }
}
