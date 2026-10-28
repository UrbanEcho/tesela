//! Atomic mutation and one-pass aggregation helpers.

use super::*;
use crate::residual::{Borrowed, Predicate};

pub(super) fn check_revision(record: &Record, expected: Option<&Revision>) -> Result<(), Error> {
    if let Some(expected) = expected
        && record.revision.as_ref() != Some(expected)
    {
        return Err(Error::stale_revision(format!(
            "expected revision '{expected}', found '{}'",
            record.revision.as_ref().map_or("none", Revision::as_str)
        )));
    }
    Ok(())
}

fn check_preconditions(record: &Record, preconditions: &[tesela_ir::Filter]) -> Result<(), Error> {
    for precondition in preconditions {
        let access = Borrowed::default();
        if !Predicate::compile(&access, precondition)?.matches(&access, &record) {
            return Err(Error::new(
                tesela_core::ErrorCode::PreconditionFailed,
                "mutation precondition was not satisfied",
            ));
        }
    }
    Ok(())
}

pub(super) fn apply_staged(
    records: &mut BTreeMap<Key, Record>,
    primary_key: &ApiName,
    mutation: &Mutation,
    next_revision: impl Fn() -> Revision + Copy,
) -> Result<i64, Error> {
    match mutation {
        Mutation::Create { values } => {
            let key = values.get(primary_key).map(Key::from).ok_or_else(|| {
                Error::bad_request(format!("memory batch create requires '{primary_key}'"))
            })?;
            if records.contains_key(&key) {
                return Err(Error::conflict(format!("record '{key}' already exists")));
            }
            let mut record = Record::new(key.clone(), values.clone());
            record.revision = Some(next_revision());
            records.insert(key, record);
            Ok(1)
        }
        Mutation::Update {
            primary_key,
            values,
            expected_revision,
            preconditions,
        } => {
            let record = records
                .get_mut(primary_key)
                .ok_or_else(|| Error::not_found("record", primary_key))?;
            check_revision(record, expected_revision.as_ref())?;
            check_preconditions(record, preconditions)?;
            record.values.extend(values.clone());
            record.revision = Some(next_revision());
            Ok(1)
        }
        Mutation::Delete {
            primary_key,
            expected_revision,
            preconditions,
        } => {
            if let Some(record) = records.get(primary_key) {
                check_revision(record, expected_revision.as_ref())?;
                check_preconditions(record, preconditions)?;
            } else if !preconditions.is_empty() {
                return Err(Error::new(
                    tesela_core::ErrorCode::PreconditionFailed,
                    "mutation subject does not exist",
                ));
            }
            Ok(i64::from(records.remove(primary_key).is_some()))
        }
        Mutation::Upsert {
            values,
            expected_revision,
            preconditions,
        } => {
            let key = values.get(primary_key).map(Key::from).ok_or_else(|| {
                Error::bad_request(format!("memory batch upsert requires '{primary_key}'"))
            })?;
            if let Some(record) = records.get(&key) {
                check_revision(record, expected_revision.as_ref())?;
                check_preconditions(record, preconditions)?;
            } else if expected_revision.is_some() {
                return Err(Error::stale_revision(
                    "record does not exist at the expected revision",
                ));
            } else if !preconditions.is_empty() {
                return Err(Error::new(
                    tesela_core::ErrorCode::PreconditionFailed,
                    "mutation subject does not exist",
                ));
            }
            let mut record = Record::new(key.clone(), values.clone());
            record.revision = Some(next_revision());
            records.insert(key, record);
            Ok(1)
        }
        Mutation::Batch { .. } => Err(Error::bad_request(
            "nested atomic batches are not supported",
        )),
    }
}

#[derive(Default)]
struct AggregateState {
    count: u64,
    numeric_count: u64,
    sum: f64,
    min: Option<Value>,
    max: Option<Value>,
}

impl AggregateState {
    fn add(&mut self, record: &Record, aggregation: &crate::Aggregation) {
        self.count += 1;
        let value = aggregation
            .property
            .as_ref()
            .and_then(|property| record.values.get(property));
        if let Some(number) = value.and_then(Value::as_f64) {
            self.sum += number;
            self.numeric_count += 1;
        }
        if let Some(value) = value {
            if self.min.as_ref().is_none_or(|current| value < current) {
                self.min = Some(value.clone());
            }
            if self.max.as_ref().is_none_or(|current| value > current) {
                self.max = Some(value.clone());
            }
        }
    }

    fn finish(&self, function: crate::AggregationFunction) -> Value {
        match function {
            crate::AggregationFunction::Count => Value::integer(self.count as i64),
            crate::AggregationFunction::Sum => Value::float(self.sum),
            crate::AggregationFunction::Avg if self.numeric_count > 0 => {
                Value::float(self.sum / self.numeric_count as f64)
            }
            crate::AggregationFunction::Avg => Value::null(),
            crate::AggregationFunction::Min => self.min.clone().unwrap_or_else(Value::null),
            crate::AggregationFunction::Max => self.max.clone().unwrap_or_else(Value::null),
        }
    }
}

pub(super) fn compute_aggregate(
    records: &[Record],
    group_by: &[ApiName],
    aggregations: &[crate::Aggregation],
) -> AggregateResult {
    let mut groups: BTreeMap<Vec<Key>, Vec<AggregateState>> = BTreeMap::new();
    for record in records {
        let key: Vec<_> = group_by
            .iter()
            .map(|field| record.values.get(field).map_or(Key::Null, Key::from))
            .collect();
        let states = groups.entry(key).or_insert_with(|| {
            (0..aggregations.len())
                .map(|_| AggregateState::default())
                .collect()
        });
        for (state, aggregation) in states.iter_mut().zip(aggregations) {
            state.add(record, aggregation);
        }
    }
    if groups.is_empty() && group_by.is_empty() {
        groups.insert(
            Vec::new(),
            (0..aggregations.len())
                .map(|_| AggregateState::default())
                .collect(),
        );
    }
    AggregateResult {
        groups: groups
            .into_iter()
            .map(|(key, states)| {
                let mut row = BTreeMap::new();
                for (field, value) in group_by.iter().zip(key) {
                    row.insert(field.to_string(), value.to_value());
                }
                for (aggregation, state) in aggregations.iter().zip(states) {
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
