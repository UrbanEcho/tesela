//! Residual filters compiled against a resolved field layout.

use tesela_core::{Error, FilterOp, KeySet, LikePlan, Value};
use tesela_ir::Filter;

use super::access::RowAccess;

/// A filter whose fields are resolved to slots and whose operands are
/// borrowed from the query.
pub(crate) enum Predicate<'q, S> {
    And(Vec<Self>),
    Or(Vec<Self>),
    Not(Box<Self>),
    Leaf(Box<Leaf<'q, S>>),
}

pub(crate) struct Leaf<'q, S> {
    op: FilterOp,
    slot: Option<S>,
    value: Option<&'q Value>,
    bounds: Option<(&'q Value, &'q Value)>,
    membership: KeySet,
    like: Option<LikePlan>,
}

impl<'q, S: Clone> Predicate<'q, S> {
    pub(crate) fn compile<A: RowAccess<Slot = S>>(
        access: &A,
        filter: &'q Filter,
    ) -> Result<Self, Error> {
        let children = |args: &'q [Filter]| {
            args.iter()
                .map(|item| Self::compile(access, item))
                .collect::<Result<Vec<_>, _>>()
        };
        match filter.op {
            FilterOp::And => Ok(Self::And(children(&filter.args)?)),
            FilterOp::Or => Ok(Self::Or(children(&filter.args)?)),
            FilterOp::Not => Ok(Self::Not(Box::new(
                filter
                    .args
                    .first()
                    .map(|item| Self::compile(access, item))
                    .transpose()?
                    .unwrap_or(Self::And(Vec::new())),
            ))),
            op => Ok(Self::Leaf(Box::new(Leaf::compile(access, op, filter)?))),
        }
    }

    pub(crate) fn matches<A: RowAccess<Slot = S>>(&self, access: &A, row: &A::Row) -> bool {
        match self {
            Self::And(items) => items.iter().all(|item| item.matches(access, row)),
            Self::Or(items) => items.iter().any(|item| item.matches(access, row)),
            Self::Not(item) => !item.matches(access, row),
            Self::Leaf(leaf) => leaf.matches(access, row),
        }
    }
}

impl<'q, S: Clone> Leaf<'q, S> {
    fn compile<A: RowAccess<Slot = S>>(
        access: &A,
        op: FilterOp,
        filter: &'q Filter,
    ) -> Result<Self, Error> {
        let membership = if matches!(op, FilterOp::In | FilterOp::NotIn) {
            filter.values.iter().map(tesela_core::Key::from).collect()
        } else {
            KeySet::default()
        };
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
        Ok(Self {
            op,
            slot: filter.field.as_ref().map(|field| access.slot(field)),
            value: filter.value.as_ref(),
            bounds: filter.values.first().zip(filter.values.get(1)),
            membership,
            like,
        })
    }

    fn matches<A: RowAccess<Slot = S>>(&self, access: &A, row: &A::Row) -> bool {
        let actual = self.slot.as_ref().and_then(|slot| access.value(row, slot));
        match self.op {
            FilterOp::In => actual.is_some_and(|item| self.membership.contains_value(item)),
            FilterOp::NotIn => actual.is_none_or(|item| !self.membership.contains_value(item)),
            FilterOp::Like => actual
                .and_then(Value::as_str)
                .is_some_and(|text| self.like.as_ref().is_some_and(|plan| plan.matches(text))),
            _ => self.evaluate(actual),
        }
    }

    fn evaluate(&self, actual: Option<&Value>) -> bool {
        let compare = |predicate: fn(&Value, &Value) -> bool| {
            self.value.map_or_else(
                || actual.is_none_or(Value::is_null),
                |expected| actual.is_some_and(|actual| predicate(actual, expected)),
            )
        };
        let strings = |predicate: fn(&str, &str) -> bool| match (
            actual.and_then(Value::as_str),
            self.value.and_then(Value::as_str),
        ) {
            (Some(actual), Some(expected)) => predicate(actual, expected),
            _ => false,
        };
        match self.op {
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
            FilterOp::EndsWith => strings(|actual, expected| actual.ends_with(expected)),
            FilterOp::Between => actual.is_some_and(|actual| {
                self.bounds
                    .is_some_and(|(low, high)| actual >= low && actual <= high)
            }),
            FilterOp::In | FilterOp::NotIn | FilterOp::Like => false,
            FilterOp::And | FilterOp::Or | FilterOp::Not => true,
        }
    }
}
