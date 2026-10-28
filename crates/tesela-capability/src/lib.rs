#![deny(warnings)]
#![deny(missing_docs)]

//! Actor-specific semantic capability projection and bounded result handles.

mod catalog;
mod diff;
mod handle;

pub use catalog::{
    Capability, CapabilityAnnotations, CapabilityCatalog, CapabilityInvocation, ToolExposure,
    contains_phrase, tokenize,
};
pub use diff::{DiffImpact, SemanticChange, semantic_diff};
pub use handle::ResultHandle;

#[cfg(test)]
mod tests;
