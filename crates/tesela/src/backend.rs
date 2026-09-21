//! What a host implements to serve ontology data.
//!
//! A backend answers for the object types a host binds to it. It is deliberately
//! small: object reads and native mutations live here, link resolution and
//! action execution use separate contracts, and the runtime owns policy,
//! validation, planning and cross-binding coordination. Implementations declare
//! their supported operations through [`BackendCapabilities`]; unsupported
//! operations fail explicitly instead of being synthesized from weaker ones.
//!
//! The reference memory backend is exposed separately through [`crate::dev`].

pub use tesela_backend::{
    ActionExecutor, Backend, BackendBuilder, BackendCapabilities, Bindings, BindingsBuilder,
    ExecutionEvent, ExecutionObserver, FnActionExecutor, FnLinkResolver, LinkResolver,
    check_backend, execute_residual,
};
