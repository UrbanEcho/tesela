//! Functional policy helpers.

use std::sync::Arc;

use crate::error::Error;
use crate::raw::{PolicyDecision, PolicyEngine, PolicyRequest};

/// Borrowed policy request.
pub type Request<'a> = PolicyRequest<'a>;
/// Policy decision enforced by the runtime.
pub type Decision = PolicyDecision;

/// Construct an unrestricted allow decision for use in a policy closure.
#[must_use]
pub fn allow() -> Decision {
    Decision {
        allow: true,
        ..Decision::default()
    }
}

/// Construct a denial with a stable human-readable reason.
#[must_use]
pub fn deny(reason: impl Into<String>) -> Decision {
    Decision {
        allow: false,
        reason: Some(reason.into()),
        ..Decision::default()
    }
}

/// Allow an operation without further restrictions.
pub fn allow_all(_request: &Request<'_>) -> Result<Decision, Error> {
    Ok(allow())
}

/// Deny an operation by default.
pub fn deny_all(_request: &Request<'_>) -> Result<Decision, Error> {
    Ok(deny("denied by policy"))
}

pub(crate) struct FunctionPolicy<F>(pub F);
impl<F> PolicyEngine for FunctionPolicy<F>
where
    F: for<'a> Fn(&PolicyRequest<'a>) -> Result<PolicyDecision, Error> + Send + Sync,
{
    fn evaluate(&self, request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error> {
        (self.0)(request)
    }
}

pub(crate) fn boxed<F>(policy: F) -> Arc<dyn PolicyEngine>
where
    F: for<'a> Fn(&PolicyRequest<'a>) -> Result<PolicyDecision, Error> + Send + Sync + 'static,
{
    Arc::new(FunctionPolicy(policy))
}
