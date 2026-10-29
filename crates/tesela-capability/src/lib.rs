#![deny(warnings)]
#![deny(missing_docs)]

//! Actor-specific semantic capability projection and bounded result handles.

mod catalog;
mod diff;
mod handle;
mod program;

pub use catalog::{
    Capability, CapabilityAnnotations, CapabilityCatalog, CapabilityInvocation, ToolExposure,
    contains_phrase, tokenize,
};
pub use diff::{DiffImpact, SemanticChange, semantic_diff};
pub use handle::ResultHandle;
pub use program::{MAX_PROGRAM_STEPS, Program, ProgramStep};

#[cfg(test)]
mod tests;
