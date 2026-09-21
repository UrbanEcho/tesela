//! Built-in policy helpers.

use tesela_backend::{PolicyDecision, PolicyEngine, PolicyRequest};
use tesela_core::Error;

/// Policy engine for trusted tests and local harnesses.
pub struct AllowAllPolicy;

impl PolicyEngine for AllowAllPolicy {
    fn evaluate(&self, _request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error> {
        Ok(PolicyDecision {
            allow: true,
            ..PolicyDecision::default()
        })
    }
}
