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

/// Unspent budget for one execution; `None` means the resource is unlimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RemainingBudget {
    /// Backend calls still allowed.
    pub backend_calls: Option<u64>,
    /// Rows still allowed.
    pub rows: Option<u64>,
    /// Bytes still allowed.
    pub bytes: Option<u64>,
    /// External cost units still allowed.
    pub external_cost: Option<u64>,
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
    /// Start an explicit context builder for one principal.
    #[must_use]
    pub fn builder(principal: impl AsRef<str>) -> ExecutionContextBuilder {
        ExecutionContextBuilder::new(principal)
    }
    /// Build and validate an execution context.
    pub fn new(
        principal: Principal,
        authority: Vec<AuthorityGrant>,
        request_id: impl AsRef<str>,
        deadline: Option<Instant>,
        cancellation: CancellationToken,
        budget: ExecutionBudget,
    ) -> Result<Self, Error> {
        if principal.id.is_empty() {
            return Err(Error::validation("principal must not be empty"));
        }
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
    /// Delegated authority chain supplied by the host.
    #[must_use]
    pub fn authority(&self) -> &[AuthorityGrant] {
        &self.authority
    }
    /// Explicitly approved semantic resources.
    #[must_use]
    pub fn approvals(&self) -> &[ApiName] {
        &self.approvals
    }
    /// Immutable resource limits for this execution.
    #[must_use]
    pub const fn budget(&self) -> &ExecutionBudget {
        &self.budget
    }
    /// Whether cancellation has been requested.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
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
    /// Budget still available for each limited resource; `None` means unlimited.
    #[must_use]
    pub fn remaining_budget(&self) -> RemainingBudget {
        let left = |used: &AtomicU64, limit: Option<u64>| {
            limit.map(|limit| limit.saturating_sub(used.load(Ordering::Relaxed)))
        };
        RemainingBudget {
            backend_calls: left(&self.usage.backend_calls, self.budget.backend_calls),
            rows: left(&self.usage.rows, self.budget.rows),
            bytes: left(&self.usage.bytes, self.budget.bytes),
            external_cost: left(&self.usage.external_cost, self.budget.external_cost),
        }
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

/// Builder for an immutable execution context.
pub struct ExecutionContextBuilder {
    principal: Principal,
    authority: Vec<AuthorityGrant>,
    approvals: Vec<ApiName>,
    request_id: Option<String>,
    deadline: Option<Instant>,
    cancellation: CancellationToken,
    budget: ExecutionBudget,
}

impl ExecutionContextBuilder {
    fn new(principal: impl AsRef<str>) -> Self {
        Self {
            principal: Principal::new(principal),
            authority: Vec::new(),
            approvals: Vec::new(),
            request_id: None,
            deadline: None,
            cancellation: CancellationToken::default(),
            budget: ExecutionBudget::default(),
        }
    }
    /// Add a principal claim.
    #[must_use]
    pub fn claim(mut self, name: impl Into<String>, value: impl Into<Value>) -> Self {
        self.principal.claims.insert(name.into(), value.into());
        self
    }
    /// Set the request identifier.
    #[must_use]
    pub fn request_id(mut self, value: impl Into<String>) -> Self {
        self.request_id = Some(value.into());
        self
    }
    /// Set delegated authority.
    #[must_use]
    pub fn authority(mut self, value: Vec<AuthorityGrant>) -> Self {
        self.authority = value;
        self
    }
    /// Add an approved semantic resource.
    #[must_use]
    pub fn approve(mut self, value: impl Into<ApiName>) -> Self {
        self.approvals.push(value.into());
        self
    }
    /// Set an absolute monotonic deadline.
    #[must_use]
    pub fn deadline(mut self, value: Instant) -> Self {
        self.deadline = Some(value);
        self
    }
    /// Set the shared cancellation handle.
    #[must_use]
    pub fn cancellation(mut self, value: CancellationToken) -> Self {
        self.cancellation = value;
        self
    }
    /// Set execution resource limits.
    #[must_use]
    pub fn budget(mut self, value: ExecutionBudget) -> Self {
        self.budget = value;
        self
    }
    /// Validate and freeze the context.
    pub fn build(self) -> Result<ExecutionContext, Error> {
        let request_id = self
            .request_id
            .ok_or_else(|| Error::validation("request_id is required"))?;
        ExecutionContext::new(
            self.principal,
            self.authority,
            request_id,
            self.deadline,
            self.cancellation,
            self.budget,
        )
        .map(|context| context.with_approvals(self.approvals))
    }
}

#[cfg(test)]
#[path = "execution_tests.rs"]
mod tests;
