//! The handle that ties an ontology, its backends and its policy together.
//!
//! [`Runtime`] compiles a [`Spec`](crate::model::Spec) once, binds its object
//! types, links and actions to backends, and serves every operation through a
//! single checked path. It owns no data and holds no connection: a host can
//! build one per tenant, per request, or once per process.
//!
//! A compiled ontology is swapped atomically — applying a spec that fails to
//! compile leaves the previous one in service — so a running host can be
//! reconfigured without a window in which nothing is valid.

pub use tesela_runtime::{
    AccessExplanation, Execution, ExecutionPlan, ExecutionReceipt, Runtime, RuntimeBuilder,
};
