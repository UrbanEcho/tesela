#![deny(warnings)]
#![deny(missing_docs)]

//! Backend contracts for Tesela ontology runtimes.
//!
//! Tesela owns the ontology contract. Hosts own concrete persistence and service integrations.

pub mod conformance;
#[cfg(any(feature = "dev", test))]
mod memory;
mod query;
mod residual;
mod traits;

#[cfg(any(feature = "dev", test))]
pub use memory::MemoryBackend;
pub use query::*;
pub use residual::{execute_residual, execute_residual_owned, execute_residual_try_owned};
pub use traits::*;
