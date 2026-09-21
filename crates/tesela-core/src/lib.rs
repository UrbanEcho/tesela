//! Core primitives for the Tesela ontology runtime.
//!
//! This crate defines identifiers, errors, diagnostics, domain enums, and value types
//! used across the entire Tesela ecosystem. It contains no business logic — only
//! strongly-typed building blocks.

#![deny(warnings)]
#![deny(missing_docs)]

pub mod diagnostics;
pub mod error;
pub mod execution;
pub mod ident;
pub mod key;
pub mod like;
pub mod sync;
pub mod types;
pub mod value;

pub use diagnostics::*;
pub use error::*;
pub use execution::*;
pub use ident::*;
pub use key::*;
pub use like::*;
pub use sync::*;
pub use types::*;
pub use value::*;
