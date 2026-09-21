//! Immutable typed expressions.

use std::marker::PhantomData;
use std::ops::{BitAnd, BitOr, Not};

use crate::model::{Field, Object};
use crate::raw::{Aggregation, AggregationFunction, Filter, FilterOp, Sort, SortDirection};

/// Boolean expression over one object type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Expr<O> {
    pub(crate) filter: Filter,
    marker: PhantomData<fn() -> O>,
}

impl<O> Expr<O> {
    pub(crate) fn new(filter: Filter) -> Self {
        Self {
            filter,
            marker: PhantomData,
        }
    }

    /// Combine two predicates with logical AND.
    #[must_use]
    pub fn and(self, rhs: Self) -> Self {
        self & rhs
    }

    /// Combine two predicates with logical OR.
    #[must_use]
    pub fn or(self, rhs: Self) -> Self {
        self | rhs
    }

    /// Negate this predicate.
    #[must_use]
    pub fn negate(self) -> Self {
        !self
    }
}

impl<O> BitAnd for Expr<O> {
    type Output = Self;
    fn bitand(self, rhs: Self) -> Self::Output {
        Self::new(Filter::and(vec![self.filter, rhs.filter]))
    }
}
impl<O> BitOr for Expr<O> {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self::new(Filter::or(vec![self.filter, rhs.filter]))
    }
}
impl<O> Not for Expr<O> {
    type Output = Self;
    fn not(self) -> Self::Output {
        Self::new(Filter::negate(self.filter))
    }
}

/// Typed ordering expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Order<O> {
    pub(crate) sort: Sort,
    marker: PhantomData<fn() -> O>,
}

/// Typed aggregation expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AggregateExpr<O> {
    pub(crate) aggregation: Aggregation,
    marker: PhantomData<fn() -> O>,
}

impl<O, T> Field<O, T> {
    /// Equality predicate.
    pub fn eq(self, value: impl Into<crate::raw::Value>) -> Expr<O> {
        Expr::new(Filter::eq(self.name(), value.into()))
    }
    /// Inequality predicate.
    pub fn ne(self, value: impl Into<crate::raw::Value>) -> Expr<O> {
        Expr::new(Filter {
            op: FilterOp::Ne,
            field: Some(self.name()),
            value: Some(value.into()),
            values: Vec::new(),
            args: Vec::new(),
        })
    }
    /// Strict less-than predicate.
    pub fn lt(self, value: impl Into<crate::raw::Value>) -> Expr<O> {
        scalar(self.name(), FilterOp::Lt, value)
    }
    /// Less-than-or-equal predicate.
    pub fn lte(self, value: impl Into<crate::raw::Value>) -> Expr<O> {
        scalar(self.name(), FilterOp::Lte, value)
    }
    /// Strict greater-than predicate.
    pub fn gt(self, value: impl Into<crate::raw::Value>) -> Expr<O> {
        scalar(self.name(), FilterOp::Gt, value)
    }
    /// Greater-than-or-equal predicate.
    pub fn gte(self, value: impl Into<crate::raw::Value>) -> Expr<O> {
        scalar(self.name(), FilterOp::Gte, value)
    }
    /// Membership predicate.
    pub fn is_in<I, V>(self, values: I) -> Expr<O>
    where
        I: IntoIterator<Item = V>,
        V: Into<crate::raw::Value>,
    {
        Expr::new(Filter::is_in(
            self.name(),
            values.into_iter().map(Into::into).collect(),
        ))
    }
    /// Null predicate.
    #[must_use]
    pub fn is_null(self) -> Expr<O> {
        Expr::new(Filter::is_null(self.name()))
    }
    /// Non-null predicate.
    #[must_use]
    pub fn is_not_null(self) -> Expr<O> {
        Expr::new(Filter::is_not_null(self.name()))
    }
    /// Ascending order.
    #[must_use]
    pub fn asc(self) -> Order<O> {
        order(self.name(), SortDirection::Asc)
    }
    /// Descending order.
    #[must_use]
    pub fn desc(self) -> Order<O> {
        order(self.name(), SortDirection::Desc)
    }
    /// Sum aggregation.
    #[must_use]
    pub fn sum(self, alias: impl Into<String>) -> AggregateExpr<O> {
        aggregate(Some(self.name()), AggregationFunction::Sum, alias)
    }
    /// Mean aggregation.
    #[must_use]
    pub fn mean(self, alias: impl Into<String>) -> AggregateExpr<O> {
        aggregate(Some(self.name()), AggregationFunction::Avg, alias)
    }
    /// Minimum aggregation.
    #[must_use]
    pub fn min(self, alias: impl Into<String>) -> AggregateExpr<O> {
        aggregate(Some(self.name()), AggregationFunction::Min, alias)
    }
    /// Maximum aggregation.
    #[must_use]
    pub fn max(self, alias: impl Into<String>) -> AggregateExpr<O> {
        aggregate(Some(self.name()), AggregationFunction::Max, alias)
    }
}

impl<O> Field<O, String> {
    /// Prefix predicate.
    pub fn starts_with(self, value: &str) -> Expr<O> {
        text(self.name(), FilterOp::StartsWith, value)
    }
    /// Suffix predicate.
    pub fn ends_with(self, value: &str) -> Expr<O> {
        text(self.name(), FilterOp::EndsWith, value)
    }
    /// Substring predicate.
    pub fn contains(self, value: &str) -> Expr<O> {
        text(self.name(), FilterOp::Contains, value)
    }
}

impl<O> Field<O, Option<String>> {
    /// Prefix predicate over a nullable string field.
    pub fn starts_with(self, value: &str) -> Expr<O> {
        text(self.name(), FilterOp::StartsWith, value)
    }
    /// Suffix predicate over a nullable string field.
    pub fn ends_with(self, value: &str) -> Expr<O> {
        text(self.name(), FilterOp::EndsWith, value)
    }
    /// Substring predicate over a nullable string field.
    pub fn contains(self, value: &str) -> Expr<O> {
        text(self.name(), FilterOp::Contains, value)
    }
}

impl<O: Object> Field<O, bool> {
    /// Treat a boolean field as a predicate equal to true.
    pub fn predicate(self) -> Expr<O> {
        Expr::new(Filter::eq(self.name(), true))
    }
}

/// Count rows under an alias.
#[must_use]
pub fn count<O>(alias: impl Into<String>) -> AggregateExpr<O> {
    aggregate(None, AggregationFunction::Count, alias)
}

fn scalar<O>(
    field: crate::raw::ApiName,
    op: FilterOp,
    value: impl Into<crate::raw::Value>,
) -> Expr<O> {
    Expr::new(Filter {
        op,
        field: Some(field),
        value: Some(value.into()),
        values: Vec::new(),
        args: Vec::new(),
    })
}
fn text<O>(field: crate::raw::ApiName, op: FilterOp, value: &str) -> Expr<O> {
    Expr::new(Filter {
        op,
        field: Some(field),
        value: Some(value.into()),
        values: Vec::new(),
        args: Vec::new(),
    })
}
fn order<O>(property: crate::raw::ApiName, direction: SortDirection) -> Order<O> {
    Order {
        sort: Sort {
            property,
            direction,
        },
        marker: PhantomData,
    }
}
fn aggregate<O>(
    property: Option<crate::raw::ApiName>,
    function: AggregationFunction,
    alias: impl Into<String>,
) -> AggregateExpr<O> {
    AggregateExpr {
        aggregation: Aggregation {
            function,
            property,
            alias: alias.into(),
        },
        marker: PhantomData,
    }
}
