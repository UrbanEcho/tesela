//! Principals, delegated authority, cancellation, deadlines, and budgets.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::{ApiName, Error, Operation, Value};

/// An authenticated human, service, or agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Principal {
    /// Stable host-defined identity.
    pub id: Arc<str>,
    /// Host-defined claims made available to policy.
    #[serde(default)]
    pub claims: BTreeMap<String, Value>,
}

impl Principal {
    /// Construct a principal without claims.
    #[must_use]
    pub fn new(id: impl AsRef<str>) -> Self {
        Self {
            id: Arc::from(id.as_ref()),
            claims: BTreeMap::new(),
        }
    }
}

/// One semantic capability granted to a delegate.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuthorityScope {
    /// Allowed operation.
    pub operation: Operation,
    /// Optional resource. `None` covers every resource for the operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<ApiName>,
}

impl AuthorityScope {
    fn contains(&self, child: &Self) -> bool {
        self.operation == child.operation
            && (self.resource.is_none() || self.resource == child.resource)
    }
}

/// A bounded delegation from one principal to another.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityGrant {
    /// Grant issuer.
    pub issuer: Arc<str>,
    /// Principal receiving the authority.
    pub delegate: Arc<str>,
    /// Allowed semantic operations.
    pub scopes: Arc<[AuthorityScope]>,
    /// Expiry as Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_unix_ms: Option<u64>,
    /// Maximum calls under this grant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_calls: Option<u64>,
    /// Maximum number of further delegations.
    pub delegation_depth: u16,
}

impl AuthorityGrant {
    /// Validate that `child` only attenuates this grant.
    pub fn validate_child(&self, child: &Self) -> Result<(), Error> {
        self.validate_time()?;
        child.validate_time()?;
        if child.issuer.as_ref() != self.delegate.as_ref() {
            return Err(Error::unauthorized(
                "delegation issuer is not the parent delegate",
            ));
        }
        if self.delegation_depth == 0 || child.delegation_depth >= self.delegation_depth {
            return Err(Error::unauthorized("delegation depth did not decrease"));
        }
        if child
            .scopes
            .iter()
            .any(|scope| !self.scopes.iter().any(|parent| parent.contains(scope)))
        {
            return Err(Error::unauthorized(
                "delegated authority exceeds its parent",
            ));
        }
        match (self.max_calls, child.max_calls) {
            (Some(parent), Some(child)) if child <= parent => {}
            (Some(_), _) => {
                return Err(Error::unauthorized(
                    "delegation removed or expanded the call budget",
                ));
            }
            _ => {}
        }
        match (self.expires_at_unix_ms, child.expires_at_unix_ms) {
            (Some(parent), Some(child)) if child <= parent => {}
            (Some(_), _) => {
                return Err(Error::unauthorized(
                    "delegation removed or expanded the expiry",
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_time(&self) -> Result<(), Error> {
        if let Some(expiry) = self.expires_at_unix_ms {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            if now >= expiry {
                return Err(Error::unauthorized("authority grant expired"));
            }
        }
        Ok(())
    }

    fn allows(&self, operation: Operation, resource: &ApiName) -> bool {
        self.scopes.iter().any(|scope| {
            scope.operation == operation
                && scope
                    .resource
                    .as_ref()
                    .is_none_or(|allowed| allowed == resource)
        })
    }
}

/// Immutable resource limits for one execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ExecutionBudget {
    /// Maximum backend calls.
    pub backend_calls: Option<u64>,
    /// Maximum rows materialized.
    pub rows: Option<u64>,
    /// Maximum bytes materialized.
    pub bytes: Option<u64>,
    /// Maximum host-reported external cost units.
    pub external_cost: Option<u64>,
    /// Maximum elapsed time for this execution.
    pub wall_time: Option<Duration>,
}

#[derive(Debug, Default)]
struct Usage {
    backend_calls: AtomicU64,
    rows: AtomicU64,
    bytes: AtomicU64,
    external_cost: AtomicU64,
}

/// Shared cancellation handle.
#[derive(Debug, Clone, Default)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    /// Request cancellation.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    /// Whether cancellation was requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

/// Authority and resource limits propagated through one request.
#[derive(Debug, Clone)]
pub struct ExecutionContext {
    principal: Principal,
    authority: Arc<[AuthorityGrant]>,
    approvals: Arc<[ApiName]>,
    request_id: Arc<str>,
    deadline: Option<Instant>,
    started: Instant,
    cancellation: CancellationToken,
    budget: ExecutionBudget,
    usage: Arc<Usage>,
}

impl ExecutionContext {
    /// Build and validate an execution context.
    pub fn new(
        principal: Principal,
        authority: Vec<AuthorityGrant>,
        request_id: impl AsRef<str>,
        deadline: Option<Instant>,
        cancellation: CancellationToken,
        budget: ExecutionBudget,
    ) -> Result<Self, Error> {
        if request_id.as_ref().is_empty() {
            return Err(Error::validation("request_id must not be empty"));
        }
        if let Some(root) = authority.first() {
            root.validate_time()?;
            for pair in authority.windows(2) {
                pair[0].validate_child(&pair[1])?;
            }
            if authority
                .last()
                .is_none_or(|leaf| leaf.delegate.as_ref() != principal.id.as_ref())
            {
                return Err(Error::unauthorized(
                    "authority chain does not terminate at the principal",
                ));
            }
        }
        Ok(Self {
            principal,
            authority: authority.into(),
            approvals: Arc::from([]),
            request_id: Arc::from(request_id.as_ref()),
            deadline,
            started: Instant::now(),
            cancellation,
            budget,
            usage: Arc::new(Usage::default()),
        })
    }

    /// Unrestricted context for a trusted host boundary.
    pub fn trusted(principal: Principal, request_id: impl AsRef<str>) -> Result<Self, Error> {
        Self::new(
            principal,
            Vec::new(),
            request_id,
            None,
            CancellationToken::default(),
            ExecutionBudget::default(),
        )
    }

    /// Authenticated principal.
    #[must_use]
    pub fn principal(&self) -> &Principal {
        &self.principal
    }
    /// Stable request identifier.
    #[must_use]
    pub fn request_id(&self) -> &str {
        &self.request_id
    }
    /// Add explicit host approvals before execution begins.
    #[must_use]
    pub fn with_approvals(mut self, approvals: Vec<ApiName>) -> Self {
        self.approvals = approvals.into();
        self
    }
    /// Whether an action/resource has explicit approval.
    #[must_use]
    pub fn is_approved(&self, resource: &ApiName) -> bool {
        self.approvals.iter().any(|item| item == resource)
    }

    /// Validate cancellation, deadline, and delegated authority.
    pub fn check(&self, operation: Operation, resource: &ApiName) -> Result<(), Error> {
        if self.cancellation.is_cancelled() {
            return Err(Error::cancelled("execution cancelled"));
        }
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(Error::timeout("execution deadline elapsed"));
        }
        if self
            .budget
            .wall_time
            .is_some_and(|limit| self.started.elapsed() >= limit)
        {
            return Err(Error::timeout("execution wall-time budget elapsed"));
        }
        if let Some(leaf) = self.authority.last() {
            leaf.validate_time()?;
            if !leaf.allows(operation, resource) {
                return Err(Error::unauthorized(
                    "delegated authority does not allow this operation",
                ));
            }
            if let Some(limit) = leaf.max_calls
                && self.usage.backend_calls.load(Ordering::Relaxed) >= limit
            {
                return Err(Error::budget_exhausted("authority call budget exhausted"));
            }
        }
        Ok(())
    }

    /// Charge one backend call.
    pub fn charge_backend_call(&self) -> Result<(), Error> {
        self.charge(
            &self.usage.backend_calls,
            1,
            self.budget.backend_calls,
            "backend calls",
        )
    }
    /// Charge materialized rows.
    pub fn charge_rows(&self, rows: u64) -> Result<(), Error> {
        self.charge(&self.usage.rows, rows, self.budget.rows, "rows")
    }
    /// Charge materialized bytes.
    pub fn charge_bytes(&self, bytes: u64) -> Result<(), Error> {
        self.charge(&self.usage.bytes, bytes, self.budget.bytes, "bytes")
    }
    /// Charge host-reported external cost.
    pub fn charge_external_cost(&self, cost: u64) -> Result<(), Error> {
        self.charge(
            &self.usage.external_cost,
            cost,
            self.budget.external_cost,
            "external cost",
        )
    }
    /// Remaining wall time, when a deadline exists.
    #[must_use]
    pub fn remaining(&self) -> Option<Duration> {
        let deadline = self
            .deadline
            .map(|deadline| deadline.saturating_duration_since(Instant::now()));
        let wall = self
            .budget
            .wall_time
            .map(|limit| limit.saturating_sub(self.started.elapsed()));
        match (deadline, wall) {
            (Some(deadline), Some(wall)) => Some(deadline.min(wall)),
            (deadline, wall) => deadline.or(wall),
        }
    }

    fn charge(
        &self,
        counter: &AtomicU64,
        amount: u64,
        limit: Option<u64>,
        name: &str,
    ) -> Result<(), Error> {
        let previous = counter.fetch_add(amount, Ordering::Relaxed);
        if limit.is_some_and(|limit| previous.saturating_add(amount) > limit) {
            counter.fetch_sub(amount, Ordering::Relaxed);
            return Err(Error::budget_exhausted(format!("{name} budget exhausted")));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budgets_fail_before_overcharging() {
        let context = ExecutionContext::new(
            Principal::new("agent"),
            Vec::new(),
            "request",
            None,
            CancellationToken::default(),
            ExecutionBudget {
                rows: Some(2),
                ..ExecutionBudget::default()
            },
        )
        .expect("context");
        assert!(context.charge_rows(2).is_ok());
        assert!(context.charge_rows(1).is_err());
    }

    #[test]
    fn delegation_must_attenuate_and_end_at_the_principal() {
        let resource = ApiName::new_unchecked("customer");
        let root = AuthorityGrant {
            issuer: Arc::from("host"),
            delegate: Arc::from("broker"),
            scopes: Arc::from([AuthorityScope {
                operation: Operation::Read,
                resource: None,
            }]),
            expires_at_unix_ms: None,
            max_calls: Some(4),
            delegation_depth: 2,
        };
        let child = AuthorityGrant {
            issuer: Arc::from("broker"),
            delegate: Arc::from("agent"),
            scopes: Arc::from([AuthorityScope {
                operation: Operation::Read,
                resource: Some(resource.clone()),
            }]),
            expires_at_unix_ms: None,
            max_calls: Some(2),
            delegation_depth: 1,
        };
        let context = ExecutionContext::new(
            Principal::new("agent"),
            vec![root.clone(), child],
            "request",
            None,
            CancellationToken::default(),
            ExecutionBudget::default(),
        )
        .expect("valid chain");
        assert!(context.check(Operation::Read, &resource).is_ok());
        let expanded = AuthorityGrant {
            max_calls: Some(5),
            ..root.clone()
        };
        assert!(root.validate_child(&expanded).is_err());
        assert!(
            ExecutionContext::new(
                Principal::new("other"),
                vec![root],
                "request",
                None,
                CancellationToken::default(),
                ExecutionBudget::default()
            )
            .is_err()
        );
    }

    #[test]
    fn cancellation_and_deadlines_fail_before_authorization() {
        let token = CancellationToken::default();
        token.cancel();
        let cancelled = ExecutionContext::new(
            Principal::new("agent"),
            Vec::new(),
            "cancelled",
            None,
            token,
            ExecutionBudget::default(),
        )
        .expect("context");
        assert!(
            cancelled
                .check(Operation::Read, &ApiName::from("customer"))
                .is_err_and(|error| error.is(crate::ErrorCode::Cancelled))
        );

        let expired = ExecutionContext::new(
            Principal::new("agent"),
            Vec::new(),
            "expired",
            Some(Instant::now() - Duration::from_millis(1)),
            CancellationToken::default(),
            ExecutionBudget::default(),
        )
        .expect("context");
        assert!(
            expired
                .check(Operation::Read, &ApiName::from("customer"))
                .is_err_and(|error| error.is(crate::ErrorCode::Timeout))
        );
    }

    #[test]
    fn byte_and_external_cost_budgets_are_bounded() {
        let context = ExecutionContext::new(
            Principal::new("agent"),
            Vec::new(),
            "cost",
            None,
            CancellationToken::default(),
            ExecutionBudget {
                bytes: Some(8),
                external_cost: Some(2),
                ..ExecutionBudget::default()
            },
        )
        .expect("context");
        assert!(context.charge_bytes(8).is_ok());
        assert!(context.charge_bytes(1).is_err());
        assert!(context.charge_external_cost(2).is_ok());
        assert!(context.charge_external_cost(1).is_err());
    }
}
