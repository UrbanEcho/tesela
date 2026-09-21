#![deny(warnings)]
#![deny(missing_docs)]

//! Tesela facade.
//!
//! Tesela is an embeddable operational ontology runtime. A host declares an
//! ontology — object types, links, traits and actions — binds those
//! declarations to its own backends, and gets queries, mutations, policy
//! enforcement and agent-facing tools over them. Tesela stores nothing itself
//! and owns no infrastructure: protocols, databases, client libraries, agents and
//! application services are the host's.
//!
//! # The modules
//!
//! * [`model`] — what the data *is*: declarations, records and scalar values.
//! * [`query`] — what a caller *asks for*: filters, sorting, mutations,
//!   aggregations and traversals.
//! * [`backend`] — what a host *implements* and binds to semantic declarations.
//! * [`policy`] — who may do it, and under what restriction.
//! * [`runtime`] — the handle that ties the above together.
//! * `capability` (optional) — actor-specific operations projected from an ontology.
//! * [`error`] — one machine-readable error type for every operation.
//!
//! [`prelude`] is a short, opinionated import list for ordinary host code.
//! Everything else is reached through its module.
//!
/// Macro expansion support. This is not an application API.
#[doc(hidden)]
pub mod __private {
    pub use serde_json;
    pub use tesela_core::*;
    pub use tesela_ir::*;
}

#[cfg(feature = "macros")]
pub use tesela_macros::{LinkType, ObjectType, TraitDef, action};

pub mod backend;
#[cfg(feature = "capability")]
pub mod capability;
pub mod error;
pub mod model;
pub mod policy;
pub mod query;
pub mod runtime;

/// Development and backend-conformance utilities.
#[cfg(feature = "dev")]
pub mod dev {
    pub use tesela_backend::MemoryBackend;
    pub use tesela_backend::conformance;
}

/// Common imports for host code.
///
/// Deliberately small: the types nearly every call site names, and nothing
/// whose meaning depends on which backend a host chose. The memory backend is
/// excluded on purpose — importing a backend should be a decision, not a
/// side effect of importing the prelude.
pub mod prelude {
    pub use crate::error::{Error, ErrorCode};
    pub use crate::model::{
        ApiName, DataType, Key, ObjectTypeDefinition, Page, Property, Record, Spec, Value,
    };
    pub use crate::policy::{ExecutionContext, Principal};
    pub use crate::query::{Filter, FilterOp, Mutation, Query, Sort, SortDirection};
    pub use crate::runtime::{Runtime, RuntimeBuilder};

    #[cfg(feature = "macros")]
    pub use crate::model::{LinkType, ObjectType, TraitDef, action};
}
