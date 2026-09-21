#![deny(warnings)]
#![deny(missing_docs)]

//! Canonical intermediate representation for Tesela ontologies.
//!
//! The IR is the *semantic* layer of Tesela: it describes entities, properties,
//! links and actions, and the query predicates over them. It describes no
//! storage. Everything a host must decide — which database serves which object
//! type, which columns back which property — lives in the host's bindings, so
//! one spec can run against any backend.
//!
//! Two representations exist on purpose:
//!
//! * the IR in this crate, which names things (`ApiName`) and is what authors
//!   write, hosts serialize and agents are shown; and
//! * the compiled representation in `tesela-runtime`, which numbers things
//!   (`ObjectTypeId`, `PropertyId`) and is what execution walks.
//!
//! The boundary between them is `tesela_runtime::compile`.

mod action;
mod data;
mod declarative;
mod filter;
mod link;
mod named_set;
mod object_type;
mod spec;

pub use action::*;
pub use data::*;
pub use declarative::*;
pub use filter::*;
pub use link::*;
pub use named_set::*;
pub use object_type::*;
pub use spec::*;
