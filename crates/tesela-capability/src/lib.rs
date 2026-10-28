#![deny(warnings)]
#![deny(missing_docs)]

//! Actor-specific semantic capability projection and bounded result handles.

mod catalog;
mod diff;
mod handle;

pub use catalog::{Capability, CapabilityCatalog, CapabilityInvocation, ToolExposure};
pub use diff::{DiffImpact, SemanticChange, semantic_diff};
pub use handle::ResultHandle;

#[cfg(test)]
mod tests;
