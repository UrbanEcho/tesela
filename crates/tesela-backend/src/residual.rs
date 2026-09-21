//! Shared allocation-aware residual query execution.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashSet};

use tesela_core::{ApiName, Error, FilterOp, Key, LikePlan, Value};
use tesela_ir::{Filter, Page, Record};

use crate::{Query, Sort, compare_records};

/// Execute filtering, ordering and pagination over borrowed records.
///
/// Backends with only a scan primitive can use this directly. Filters are
/// compiled once, unsorted queries stop after `limit + 1` matches, and sorted
/// queries retain only the bounded top-k set before cloning returned rows.
#[inline]
pub fn execute_residual<'a>(
    records: impl IntoIterator<Item = &'a Record>,
    query: &Query,
) -> Result<Page, Error> {
    let offset = resolve_offset(query);
    let limit = query.limit.unwrap_or(1000).max(0) as usize;
    let filter = query
        .filter
        .as_ref()
        .map(CompiledResidualFilter::compile)
        .transpose()?;
    let records = records.into_iter();
    if query.sort.is_empty() {
        return Ok(match filter.as_ref() {
            Some(filter) => unsorted_page(
                records.filter(|record| filter.matches(&record.values)),
                offset,
                limit,
            ),
            None => unsorted_page(records, offset, limit),
        });
    }
    let matches = records.filter(|record| {
        filter
            .as_ref()
            .is_none_or(|item| item.matches(&record.values))
    });
    let keep = offset.saturating_add(limit).saturating_add(1);
    let mut heap = BinaryHeap::with_capacity(keep);
    for record in matches {
        let entry = RankedRecord {
            record,
            sort: &query.sort,
        };
        if heap.len() < keep {
            heap.push(entry);
        } else if heap.peek().is_some_and(|worst| entry < *worst) {
            heap.pop();
            heap.push(entry);
        }
    }
    let rows = heap.into_sorted_vec();
    let has_more = rows.len() > offset.saturating_add(limit);
    Ok(Page {
        records: rows
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|item| item.record.clone())
            .collect(),
        next_cursor: has_more.then(|| (offset + limit).to_string()),
    })
}

#[inline]
fn unsorted_page<'a>(
    records: impl Iterator<Item = &'a Record>,
    offset: usize,
    limit: usize,
) -> Page {
    let mut rows: Vec<_> = records
        .skip(offset)
        .take(limit.saturating_add(1))
        .cloned()
        .collect();
    let has_more = rows.len() > limit;
    rows.truncate(limit);
    Page {
        records: rows,
        next_cursor: has_more.then(|| (offset + limit).to_string()),
    }
}

fn resolve_offset(query: &Query) -> usize {
    query
        .cursor
        .as_deref()
        .and_then(|cursor| cursor.parse().ok())
        .or_else(|| query.offset.map(|offset| offset.max(0) as usize))
        .unwrap_or_default()
}

#[derive(Clone, Copy)]
struct RankedRecord<'a> {
    record: &'a Record,
    sort: &'a [Sort],
}
impl PartialEq for RankedRecord<'_> {
    fn eq(&self, other: &Self) -> bool {
        compare_records(self.record, other.record, self.sort).is_eq()
    }
}
impl Eq for RankedRecord<'_> {}
impl PartialOrd for RankedRecord<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for RankedRecord<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_records(self.record, other.record, self.sort)
    }
}

pub(crate) enum CompiledResidualFilter {
    And(Vec<Self>),
    Or(Vec<Self>),
    Not(Box<Self>),
    Scalar {
        op: FilterOp,
        field: Option<ApiName>,
        value: Option<Value>,
        values: Vec<Value>,
        membership: Option<HashSet<Key>>,
        like: Option<LikePlan>,
    },
}

impl CompiledResidualFilter {
    pub(crate) fn compile(filter: &Filter) -> Result<Self, Error> {
        match filter.op {
            FilterOp::And => Ok(Self::And(
                filter
                    .args
                    .iter()
                    .map(Self::compile)
                    .collect::<Result<_, _>>()?,
            )),
            FilterOp::Or => Ok(Self::Or(
                filter
                    .args
                    .iter()
                    .map(Self::compile)
                    .collect::<Result<_, _>>()?,
            )),
            FilterOp::Not => Ok(Self::Not(Box::new(
                filter
                    .args
                    .first()
                    .map(Self::compile)
                    .transpose()?
                    .unwrap_or(Self::And(Vec::new())),
            ))),
            op => {
                let membership = matches!(op, FilterOp::In | FilterOp::NotIn)
                    .then(|| filter.values.iter().map(Key::from).collect());
                let like = (op == FilterOp::Like)
                    .then(|| {
                        filter
                            .value
                            .as_ref()
                            .and_then(Value::as_str)
                            .ok_or_else(|| Error::bad_request("LIKE requires a string pattern"))
                            .and_then(|pattern| {
                                LikePlan::new(pattern)
                                    .map_err(|error| Error::bad_request(error.to_string()))
                            })
                    })
                    .transpose()?;
                Ok(Self::Scalar {
                    op,
                    field: filter.field.clone(),
                    value: filter.value.clone(),
                    values: filter.values.clone(),
                    membership,
                    like,
                })
            }
        }
    }

    pub(crate) fn matches(&self, record: &BTreeMap<ApiName, Value>) -> bool {
        match self {
            Self::And(items) => items.iter().all(|item| item.matches(record)),
            Self::Or(items) => items.iter().any(|item| item.matches(record)),
            Self::Not(item) => !item.matches(record),
            Self::Scalar {
                op,
                field,
                value,
                values,
                membership,
                like,
            } => {
                let actual = field.as_ref().and_then(|field| record.get(field));
                match op {
                    FilterOp::In => actual.is_some_and(|item| {
                        membership
                            .as_ref()
                            .is_some_and(|set| set.contains(&Key::from(item)))
                    }),
                    FilterOp::NotIn => actual.is_none_or(|item| {
                        membership
                            .as_ref()
                            .is_none_or(|set| !set.contains(&Key::from(item)))
                    }),
                    FilterOp::Like => actual
                        .and_then(Value::as_str)
                        .is_some_and(|text| like.as_ref().is_some_and(|plan| plan.matches(text))),
                    _ => evaluate_scalar(*op, actual, value.as_ref(), values),
                }
            }
        }
    }
}

fn evaluate_scalar(
    op: FilterOp,
    actual: Option<&Value>,
    expected: Option<&Value>,
    values: &[Value],
) -> bool {
    let compare = |predicate: fn(&Value, &Value) -> bool| {
        expected.map_or_else(
            || actual.is_none_or(Value::is_null),
            |expected| actual.is_some_and(|actual| predicate(actual, expected)),
        )
    };
    let strings = |predicate: fn(&str, &str) -> bool| match (
        actual.and_then(Value::as_str),
        expected.and_then(Value::as_str),
    ) {
        (Some(actual), Some(expected)) => predicate(actual, expected),
        _ => false,
    };
    match op {
        FilterOp::IsNull => actual.is_none_or(Value::is_null),
        FilterOp::IsNotNull => actual.is_some_and(|value| !value.is_null()),
        FilterOp::Eq => compare(|left, right| left == right),
        FilterOp::Ne => compare(|left, right| left != right),
        FilterOp::Lt => compare(|left, right| left < right),
        FilterOp::Lte => compare(|left, right| left <= right),
        FilterOp::Gt => compare(|left, right| left > right),
        FilterOp::Gte => compare(|left, right| left >= right),
        FilterOp::Contains => strings(|actual, expected| actual.contains(expected)),
        FilterOp::StartsWith => strings(|actual, expected| actual.starts_with(expected)),
        FilterOp::Between => actual.is_some_and(|actual| {
            values
                .first()
                .zip(values.get(1))
                .is_some_and(|(low, high)| actual >= low && actual <= high)
        }),
        FilterOp::In | FilterOp::NotIn | FilterOp::Like => false,
        FilterOp::And | FilterOp::Or | FilterOp::Not => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_scan_clones_only_the_window_and_reports_more() {
        let records: Vec<_> = (0..5)
            .map(|id| Record::new(Key::I64(id), BTreeMap::new()))
            .collect();
        let page = execute_residual(
            &records,
            &Query {
                limit: Some(2),
                ..Query::default()
            },
        )
        .expect("residual query");
        assert_eq!(page.records.len(), 2);
        assert_eq!(page.next_cursor.as_deref(), Some("2"));
    }

    #[test]
    fn membership_and_like_compile_once_and_match() {
        let field = ApiName::from("name");
        let records = [Record::new(
            Key::I64(1),
            BTreeMap::from([(field.clone(), Value::string("tesela"))]),
        )];
        for filter in [
            Filter::is_in(field.clone(), vec![Value::string("tesela")]),
            Filter {
                op: FilterOp::Like,
                field: Some(field.clone()),
                value: Some(Value::string("tes%")),
                values: Vec::new(),
                args: Vec::new(),
            },
        ] {
            assert_eq!(
                execute_residual(
                    &records,
                    &Query {
                        filter: Some(filter),
                        ..Query::default()
                    }
                )
                .expect("residual query")
                .records
                .len(),
                1
            );
        }
    }
}
