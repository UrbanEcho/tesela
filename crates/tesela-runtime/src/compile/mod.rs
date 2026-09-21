//! Compilation of a semantic [`Spec`] into an executable ontology.
//!
//! The IR is what an author writes: names, traits, references, defaults. It is
//! pleasant to author and expensive to execute — every field is a `String` key
//! comparison and every reference is a scan. Compilation resolves all of that
//! once, into a form with dense numeric handles, flattened traits and validated
//! references, so the execution path never looks a name up and never has to
//! decide what to do about a dangling reference.
//!
//! Three properties of this pass are deliberate:
//!
//! * **It is total.** Trait flattening materializes trait properties into the
//!   object type that declares them, so `customer.email` is one lookup whether
//!   `email` was declared directly or inherited. There is no inheritance walk at
//!   runtime.
//! * **It collects rather than fails fast.** One compile reports every problem
//!   in the spec, as [`Diagnostic`]s, instead of one per attempt.
//! * **It fails closed.** A spec with an error-level diagnostic produces no
//!   [`CompiledOntology`] at all. A partially compiled ontology would be a
//!   runtime that quietly ignores half its own spec.

mod filter;
mod ids;

pub use filter::{CompiledFilter, FieldRef, compile_filter};
pub use ids::{ActionId, LinkId, ObjectTypeId, PropertyId};
// `LIKE` matching is a semantic rule, so its implementation lives in
// `tesela-core` where every backend can share it. Re-exported here because this
// is where a caller compiling a filter looks for it.
pub use tesela_core::like::{LikeError, LikePlan};

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use tesela_core::{
    ApiName, DataType, Diagnostic, DiagnosticCode, Diagnostics, Error, LinkCardinality, RiskLevel,
    Value,
};
use tesela_ir::{
    ActionEffects, ApprovalPolicy, Concurrency, Filter, LinkType, ObjectType, Property,
    SPEC_VERSION, SemanticFingerprint, Spec, duplicate_names,
};

mod compiler;
mod entity;
mod lower;
mod ontology;
mod query;
mod relation;
mod validation;

pub use entity::*;
pub use ontology::*;
pub use query::*;
pub use relation::*;

#[cfg(test)]
pub(crate) mod tests;
