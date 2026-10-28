#![deny(warnings)]
#![deny(missing_docs)]

//! Ontology runtime for Tesela.
//!
//! The runtime knows about ontology metadata, policy, and backend routing. It
//! does not own transports, cloud adapters, agents, workflows, or schedulers.

pub use tesela_capability as capability;
pub mod compile;
mod policy;
mod runtime;
#[cfg(test)]
mod runtime_capability_tests;
#[cfg(test)]
mod runtime_tests;

pub use policy::AllowAllPolicy;
pub use runtime::{
    AccessExplanation, Execution, ExecutionPlan, ExecutionReceipt, Runtime, RuntimeBuilder,
};
