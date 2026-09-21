//! Immutable typed execution plans.

use std::collections::BTreeMap;
use std::marker::PhantomData;

use serde::Serialize;

use crate::error::Error;
use crate::model::{
    Action, AggregationResult, BatchResult, Execution, Field, Link, Object, Page, encode_key,
    encode_value,
};
use crate::query::{AggregateExpr, Expr, Order};
use crate::raw::{self, ApiName, Filter, Mutation, Revision, Value};

mod sealed {
    pub trait Sealed {}
}
mod action;
mod batch;
pub use batch::{Batch, BatchItem, batch};

/// A value that can be executed only by a [`Runtime`](crate::Runtime).
pub trait Plan: sealed::Sealed + Sized {
    /// Successful result.
    type Output;
    #[doc(hidden)]
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<Self::Output>, Error>;
}

/// Immutable object query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query<T: Object> {
    pub(crate) raw: raw::Query,
    marker: PhantomData<fn() -> T>,
}

/// Start a typed query.
#[must_use]
pub fn query<T: Object>() -> Query<T> {
    Query {
        raw: raw::Query::default(),
        marker: PhantomData,
    }
}

impl<T: Object> Query<T> {
    /// Add predicates with AND semantics.
    #[must_use]
    pub fn filter(mut self, predicate: Expr<T>) -> Self {
        self.raw = self.raw.and_filter(predicate.filter);
        self
    }
    /// Add predicates with AND semantics.
    #[must_use]
    pub fn where_(self, predicate: Expr<T>) -> Self {
        self.filter(predicate)
    }
    /// Add an ordering expression.
    #[must_use]
    pub fn order_by(mut self, order: Order<T>) -> Self {
        self.raw.sort.push(order.sort);
        self
    }
    /// Bound the page size.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.raw.limit = Some(limit.min(i32::MAX as usize) as i32);
        self
    }
    /// Apply an offset.
    #[must_use]
    pub fn offset(mut self, offset: usize) -> Self {
        self.raw.offset = Some(offset.min(i32::MAX as usize) as i32);
        self
    }
    /// Begin grouped aggregation.
    #[must_use]
    pub fn group_by<U>(self, field: Field<T, U>) -> Aggregate<T> {
        Aggregate {
            raw: raw::AggregateQuery {
                filter: self.raw.filter,
                group_by: vec![field.name()],
                ..Default::default()
            },
            marker: PhantomData,
        }
    }
}

impl<T: Object> sealed::Sealed for Query<T> {}
impl<T: Object> Plan for Query<T> {
    type Output = Page<T>;
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<Self::Output>, Error> {
        let execution = runtime.search(context, &T::api_name(), self.raw)?;
        let items = execution
            .output
            .records
            .into_iter()
            .map(T::decode)
            .collect::<Result<_, _>>()?;
        Ok(Execution {
            value: Page {
                items,
                next_cursor: execution.output.next_cursor,
            },
            receipt: execution.receipt,
            revision: None,
        })
    }
}

/// Primary-key lookup plan.
#[derive(Debug, Clone)]
pub struct Get<T: Object> {
    key: T::Key,
}
/// Build a primary-key lookup.
#[must_use]
pub fn get<T: Object>(key: T::Key) -> Get<T> {
    Get { key }
}
impl<T: Object> sealed::Sealed for Get<T> {}
impl<T: Object> Plan for Get<T> {
    type Output = Option<T>;
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<Self::Output>, Error> {
        let execution = runtime.lookup(context, &T::api_name(), &encode_key(&self.key)?)?;
        let revision = execution
            .output
            .as_ref()
            .and_then(|record| record.revision.clone());
        let value = execution.output.map(T::decode).transpose()?;
        Ok(Execution {
            value,
            receipt: execution.receipt,
            revision,
        })
    }
}

/// Create plan.
#[derive(Debug, Clone)]
pub struct Create<T: Object> {
    value: T,
}
/// Build a create plan.
#[must_use]
pub fn create<T: Object>(value: T) -> Create<T> {
    Create { value }
}

/// Upsert plan.
#[derive(Debug, Clone)]
pub struct Upsert<T: Object> {
    value: T,
    revision: Option<Revision>,
    preconditions: Vec<Filter>,
}
/// Build an upsert plan.
#[must_use]
pub fn upsert<T: Object>(value: T) -> Upsert<T> {
    Upsert {
        value,
        revision: None,
        preconditions: Vec::new(),
    }
}
impl<T: Object> Upsert<T> {
    /// Require a revision.
    #[must_use]
    pub fn if_revision(mut self, revision: Revision) -> Self {
        self.revision = Some(revision);
        self
    }
    /// Require a semantic predicate.
    #[must_use]
    pub fn require(mut self, predicate: Expr<T>) -> Self {
        self.preconditions.push(predicate.filter);
        self
    }
}

/// Typed update plan.
#[derive(Debug, Clone)]
pub struct Update<T: Object> {
    key: T::Key,
    values: BTreeMap<ApiName, Value>,
    revision: Option<Revision>,
    preconditions: Vec<Filter>,
    error: Option<Error>,
}
/// Build an update plan.
#[must_use]
pub fn update<T: Object>(key: T::Key) -> Update<T> {
    Update {
        key,
        values: BTreeMap::new(),
        revision: None,
        preconditions: Vec::new(),
        error: None,
    }
}
impl<T: Object> Update<T> {
    /// Set one typed field.
    #[must_use]
    pub fn set<U: Serialize>(mut self, field: Field<T, U>, value: U) -> Self {
        match encode_value(&value) {
            Ok(value) => {
                self.values.insert(field.name(), value);
            }
            Err(error) => self.error = Some(error),
        }
        self
    }
    /// Require a semantic predicate.
    #[must_use]
    pub fn require(mut self, predicate: Expr<T>) -> Self {
        self.preconditions.push(predicate.filter);
        self
    }
    /// Require a backend revision.
    #[must_use]
    pub fn if_revision(mut self, revision: Revision) -> Self {
        self.revision = Some(revision);
        self
    }
}

/// Typed delete plan.
#[derive(Debug, Clone)]
pub struct Delete<T: Object> {
    key: T::Key,
    revision: Option<Revision>,
    preconditions: Vec<Filter>,
}
/// Build a delete plan.
#[must_use]
pub fn delete<T: Object>(key: T::Key) -> Delete<T> {
    Delete {
        key,
        revision: None,
        preconditions: Vec::new(),
    }
}
impl<T: Object> Delete<T> {
    /// Require a semantic predicate.
    #[must_use]
    pub fn require(mut self, predicate: Expr<T>) -> Self {
        self.preconditions.push(predicate.filter);
        self
    }
    /// Require a backend revision.
    #[must_use]
    pub fn if_revision(mut self, revision: Revision) -> Self {
        self.revision = Some(revision);
        self
    }
}

fn mutation_record<T: Object>(
    execution: raw::RawExecution<raw::MutationResult>,
) -> Result<Execution<T>, Error> {
    let record = execution
        .output
        .record
        .ok_or_else(|| Error::adapter("typed mutation backend did not return the stored record"))?;
    let revision = record.revision.clone();
    Ok(Execution {
        value: T::decode(record)?,
        receipt: execution.receipt,
        revision,
    })
}

impl<T: Object> sealed::Sealed for Create<T> {}
impl<T: Object> Plan for Create<T> {
    type Output = T;
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<T>, Error> {
        mutation_record(runtime.mutate(
            context,
            &T::api_name(),
            Mutation::Create {
                values: self.value.encode()?,
            },
        )?)
    }
}
impl<T: Object> sealed::Sealed for Upsert<T> {}
impl<T: Object> Plan for Upsert<T> {
    type Output = T;
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<T>, Error> {
        mutation_record(runtime.mutate(
            context,
            &T::api_name(),
            Mutation::Upsert {
                values: self.value.encode()?,
                expected_revision: self.revision,
                preconditions: self.preconditions,
            },
        )?)
    }
}
impl<T: Object> sealed::Sealed for Update<T> {}
impl<T: Object> Plan for Update<T> {
    type Output = T;
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<T>, Error> {
        if let Some(error) = self.error {
            return Err(error);
        }
        mutation_record(runtime.mutate(
            context,
            &T::api_name(),
            Mutation::Update {
                primary_key: encode_key(&self.key)?,
                values: self.values,
                expected_revision: self.revision,
                preconditions: self.preconditions,
            },
        )?)
    }
}
impl<T: Object> sealed::Sealed for Delete<T> {}
impl<T: Object> Plan for Delete<T> {
    type Output = ();
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<()>, Error> {
        let execution = runtime.mutate(
            context,
            &T::api_name(),
            Mutation::Delete {
                primary_key: encode_key(&self.key)?,
                expected_revision: self.revision,
                preconditions: self.preconditions,
            },
        )?;
        Ok(Execution {
            value: (),
            receipt: execution.receipt,
            revision: None,
        })
    }
}

/// Link traversal plan.
#[derive(Debug, Clone)]
pub struct Traverse<L: Link> {
    source_key: <L::Source as Object>::Key,
    filter: Option<Filter>,
    sort: Vec<raw::Sort>,
    limit: Option<i32>,
    offset: Option<i32>,
    marker: PhantomData<fn() -> L>,
}
/// Build a typed traversal.
pub fn traverse<L: Link>(key: <L::Source as Object>::Key) -> Traverse<L> {
    Traverse {
        source_key: key,
        filter: None,
        sort: Vec::new(),
        limit: None,
        offset: None,
        marker: PhantomData,
    }
}
impl<L: Link> Traverse<L> {
    /// Add a target predicate.
    #[must_use]
    pub fn filter(mut self, predicate: Expr<L::Target>) -> Self {
        self.filter = Some(match self.filter.take() {
            Some(old) => Filter::and(vec![old, predicate.filter]),
            None => predicate.filter,
        });
        self
    }
    /// Add target ordering.
    #[must_use]
    pub fn order_by(mut self, order: Order<L::Target>) -> Self {
        self.sort.push(order.sort);
        self
    }
    /// Bound results.
    #[must_use]
    pub fn limit(mut self, limit: usize) -> Self {
        self.limit = Some(limit.min(i32::MAX as usize) as i32);
        self
    }
}
impl<L: Link> sealed::Sealed for Traverse<L> {}
impl<L: Link> Plan for Traverse<L> {
    type Output = Page<L::Target>;
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<Self::Output>, Error> {
        let raw = raw::TraversalQuery {
            source_pk: encode_key(&self.source_key)?,
            filter: self.filter,
            sort: self.sort,
            limit: self.limit,
            offset: self.offset,
        };
        let execution = runtime.traverse(context, &L::definition().api_name, raw)?;
        let items = execution
            .output
            .records
            .into_iter()
            .map(L::Target::decode)
            .collect::<Result<_, _>>()?;
        Ok(Execution {
            value: Page {
                items,
                next_cursor: execution.output.next_cursor,
            },
            receipt: execution.receipt,
            revision: None,
        })
    }
}

/// Aggregation plan.
#[derive(Debug, Clone)]
pub struct Aggregate<T: Object> {
    raw: raw::AggregateQuery,
    marker: PhantomData<fn() -> T>,
}
impl<T: Object> Aggregate<T> {
    /// Add another group key.
    #[must_use]
    pub fn group_by<U>(mut self, field: Field<T, U>) -> Self {
        self.raw.group_by.push(field.name());
        self
    }
    /// Add aggregate outputs.
    #[must_use]
    pub fn agg(mut self, expressions: impl IntoIterator<Item = AggregateExpr<T>>) -> Self {
        self.raw
            .aggregations
            .extend(expressions.into_iter().map(|item| item.aggregation));
        self
    }
}
impl<T: Object> sealed::Sealed for Aggregate<T> {}
impl<T: Object> Plan for Aggregate<T> {
    type Output = AggregationResult;
    fn execute(
        self,
        runtime: &raw::Runtime,
        context: &crate::Context,
    ) -> Result<Execution<Self::Output>, Error> {
        let execution = runtime.aggregate(context, &T::api_name(), self.raw)?;
        Ok(Execution {
            value: AggregationResult {
                groups: execution.output.groups,
            },
            receipt: execution.receipt,
            revision: None,
        })
    }
}
