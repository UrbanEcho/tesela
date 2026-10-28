#![deny(warnings)]
#![deny(missing_docs)]

//! Typed, policy-checked plans over host-owned data and services.
//!
//! Definitions and plans are immutable. [`Runtime`] is the sole execution
//! boundary, and every execution requires an explicit [`Context`].

#[doc(hidden)]
pub mod __private {
    pub use serde_json;
    pub use tesela_core::*;
    pub use tesela_ir::*;
}

pub mod backend;
pub mod error;
mod lazy;
pub mod model;
pub mod plan;
pub mod policy;
pub mod query;
pub mod raw;
pub mod runtime;

#[cfg(feature = "macros")]
pub use tesela_macros::{Action, Link, Object, Trait};

pub use backend::{MemoryBackend, backend};
pub use error::{Error, ErrorCode};
pub use model::{
    Action, AggregationResult, BatchResult, Execution, Field, Link, Object, Page, Schema, Trait,
};
pub use plan::{
    Aggregate, Batch, BatchItem, Create, Delete, Get, Plan, Query, Traverse, Update, Upsert, batch,
    create, delete, get, query, traverse, update, upsert,
};
pub use query::{AggregateExpr, Expr, Order, count};
pub use runtime::{Runtime, RuntimeBuilder};
pub use tesela_core::{
    CancellationToken, ExecutionBudget as Budget, ExecutionContext as Context,
    ExecutionContextBuilder as ContextBuilder,
};

/// Common imports for application code.
pub mod prelude {
    pub use crate::{
        Action, Context, Error, Field, Link, MemoryBackend, Object, Plan, Runtime, backend, count,
        create, delete, get, query, traverse, update, upsert,
    };
}
