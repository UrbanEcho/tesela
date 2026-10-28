#![deny(warnings)]
#![deny(missing_docs)]

//! Backend contracts for Tesela ontology runtimes.
//!
//! Tesela owns the ontology contract. Hosts own concrete persistence and service integrations.

mod aggregate;
pub mod conformance;
#[cfg(any(feature = "dev", test))]
mod memory;
mod query;
mod residual;
mod traits;

pub use aggregate::AggregateAccumulator;
#[cfg(any(feature = "dev", test))]
pub use memory::MemoryBackend;
pub use query::*;
pub use residual::{
    RowAccess, RowPage, execute_residual, execute_residual_owned, execute_residual_try_owned,
    execute_rows,
};
pub use traits::*;
