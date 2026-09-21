//! Who may do it, and under what restriction.
//!
//! Policy is a decision the runtime asks for and then *enforces*: a
//! [`PolicyDecision`] can deny an operation, narrow it with a row filter, or
//! redact properties from what comes back. The runtime applies every part of
//! that decision on every path that returns records, so a policy cannot be
//! bypassed by choosing a different call.
//!
//! [`AllowAllPolicy`] and [`DenyAllPolicy`] are the two trivial engines, useful
//! for tests and for hosts that enforce access outside Tesela. Anything real
//! implements [`PolicyEngine`].

pub use tesela_backend::{DenyAllPolicy, PolicyDecision, PolicyEngine, PolicyRequest};
pub use tesela_core::Operation;
pub use tesela_core::{
    AuthorityGrant, AuthorityScope, CancellationToken, ExecutionBudget, ExecutionContext, Principal,
};
pub use tesela_runtime::AllowAllPolicy;
