//! What a caller asks for.
//!
//! Every request is a value: it can be built, inspected, logged and replayed
//! without touching a backend, and it is checked against the compiled ontology
//! before it reaches one. A request that names a property the object type does
//! not declare fails rather than silently matching everything.
//!
//! These shapes are what the runtime compiles into a plan, and what the agent
//! tool surface accepts, so a filter an agent produces and a filter a host
//! produces take the same path.

use std::marker::PhantomData;

use crate::model::{ApiName, Value};

/// Typed descriptor generated for an object field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Field<T> {
    name: &'static str,
    marker: PhantomData<fn() -> T>,
}

impl<T> Field<T> {
    /// Construct a descriptor in generated code.
    #[doc(hidden)]
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            marker: PhantomData,
        }
    }
    /// Semantic property name.
    #[must_use]
    pub fn api_name(self) -> ApiName {
        ApiName::new_unchecked(self.name)
    }
    /// Equality predicate.
    #[must_use]
    pub fn eq(self, value: impl Into<Value>) -> Filter {
        Filter::eq(self.api_name(), value.into())
    }
    /// Membership predicate.
    #[must_use]
    pub fn is_in(self, values: impl IntoIterator<Item = impl Into<Value>>) -> Filter {
        Filter::is_in(
            self.api_name(),
            values.into_iter().map(Into::into).collect(),
        )
    }
}

pub use tesela_backend::{
    ActionRequest, AggregateQuery, Aggregation, AggregationFunction, Mutation, Query, Sort,
    SortDirection, TraversalQuery,
};
pub use tesela_core::FilterOp;
pub use tesela_ir::Filter;
