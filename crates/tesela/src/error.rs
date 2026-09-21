//! One machine-readable error type for every operation.
//!
//! An error carries a stable [`ErrorCode`], a broad [`ErrorCategory`], whether
//! it is worth retrying, and — the part that matters when a call may have
//! already reached a backend — a [`SideEffectStatus`] saying whether the
//! operation started, completed or is unknown. A caller never has to guess
//! whether a failed mutation took effect.
//!
//! Spec compilation and record validation report through [`Diagnostics`], so a
//! rejected ontology reaches the author as a list of located problems rather
//! than the first one found.

pub use tesela_core::{
    Diagnostic, DiagnosticCode, DiagnosticLevel, Diagnostics, Error, ErrorCategory, ErrorCode,
    Recovery, ResourceKind, SideEffectStatus,
};
