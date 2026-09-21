use std::fmt;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::{ErrorCategory, ErrorCode, Recovery, ResourceKind, SideEffectStatus};
use crate::diagnostics::Diagnostic;

/// The primary error type for Tesela operations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Error {
    code: ErrorCode,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    resource_kind: Option<ResourceKind>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    source_msg: Option<String>,
    #[serde(default)]
    side_effect: SideEffectStatus,
    #[serde(default)]
    retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    retry_after: Option<Duration>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    recovery: Option<Recovery>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    diagnostics: Vec<Diagnostic>,
}

impl Error {
    /// Create an error from a code and message, taking retryability and the
    /// side-effect status from the code's defaults.
    pub fn new<M: Into<String>>(code: ErrorCode, message: M) -> Self {
        Self {
            code,
            message: message.into(),
            resource_kind: None,
            resource: None,
            source_msg: None,
            // A rejection is the default: most errors are raised before any
            // write is attempted, and claiming `None` for a write path would be
            // a lie. Callers that know better set it explicitly.
            side_effect: SideEffectStatus::NotStarted,
            retryable: code.is_retryable(),
            retry_after: None,
            recovery: None,
            diagnostics: Vec::new(),
        }
    }

    /// The machine-readable code.
    #[must_use]
    pub fn code(&self) -> ErrorCode {
        self.code
    }

    /// The coarse category.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        self.code.category()
    }

    /// The human-readable message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// The kind of resource this error concerns, if any.
    #[must_use]
    pub fn resource_kind(&self) -> Option<ResourceKind> {
        self.resource_kind
    }

    /// The resource identifier this error concerns, if any.
    #[must_use]
    pub fn resource(&self) -> Option<&str> {
        self.resource.as_deref()
    }

    /// The underlying backend or adapter message, if any.
    #[must_use]
    pub fn source_message(&self) -> Option<&str> {
        self.source_msg.as_deref()
    }

    /// Whether the failed operation changed stored state.
    #[must_use]
    pub fn side_effect(&self) -> SideEffectStatus {
        self.side_effect
    }

    /// Whether retrying the identical request could plausibly succeed.
    ///
    /// This is only `true` when the code is retryable *and* the operation left
    /// no state behind.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.retryable && self.side_effect.allows_retry()
    }

    /// A server-suggested delay before retrying.
    #[must_use]
    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// The suggested recovery action.
    #[must_use]
    pub fn recovery(&self) -> Option<&Recovery> {
        self.recovery.as_ref()
    }

    /// Structured diagnostics, when this error came from a validation pass.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Whether this error carries the given code.
    #[must_use]
    pub fn is(&self, code: ErrorCode) -> bool {
        self.code == code
    }

    /// Attach the resource this error concerns.
    #[must_use]
    pub fn with_resource<K: Into<String>>(mut self, kind: ResourceKind, resource: K) -> Self {
        self.resource_kind = Some(kind);
        self.resource = Some(resource.into());
        self
    }

    /// Attach an underlying backend message.
    #[must_use]
    pub fn with_source<M: Into<String>>(mut self, source: M) -> Self {
        self.source_msg = Some(source.into());
        self
    }

    /// Declare what happened to stored state before the failure.
    #[must_use]
    pub fn with_side_effect(mut self, status: SideEffectStatus) -> Self {
        self.side_effect = status;
        self
    }

    /// Override the retryability derived from the code.
    #[must_use]
    pub fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// Suggest a delay before retrying.
    #[must_use]
    pub fn with_retry_after(mut self, delay: Duration) -> Self {
        self.retry_after = Some(delay);
        self
    }

    /// Suggest a recovery action.
    #[must_use]
    pub fn with_recovery(mut self, recovery: Recovery) -> Self {
        self.recovery = Some(recovery);
        self
    }

    /// Attach structured diagnostics.
    #[must_use]
    pub fn with_diagnostics(mut self, diagnostics: Vec<Diagnostic>) -> Self {
        self.diagnostics = diagnostics;
        self
    }

    // -- Shorthand constructors -------------------------------------------
    //
    // These keep call sites terse; they are the only sanctioned way to build
    // the common errors.

    /// A validation failure.
    pub fn validation<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::Validation, message)
    }

    /// A validation failure with structured diagnostics.
    pub fn invalid<M: Into<String>>(message: M, diagnostics: Vec<Diagnostic>) -> Self {
        Self::new(ErrorCode::Validation, message).with_diagnostics(diagnostics)
    }

    /// A resource was not found.
    pub fn not_found<K: fmt::Display, R: fmt::Display>(resource_kind: K, resource: R) -> Self {
        Self::new(
            ErrorCode::NotFound,
            format!("{resource_kind} '{resource}' not found"),
        )
    }

    /// An object type does not exist.
    pub fn unknown_object_type<R: fmt::Display>(resource: R) -> Self {
        Self::new(
            ErrorCode::UnknownObjectType,
            format!("unknown object type '{resource}'"),
        )
        .with_resource(ResourceKind::ObjectType, resource.to_string())
        .with_recovery(Recovery::Rediscover)
    }

    /// A property does not exist on an object type.
    pub fn unknown_property<O: fmt::Display, P: fmt::Display>(object: O, property: P) -> Self {
        Self::new(
            ErrorCode::UnknownProperty,
            format!("object type '{object}' has no property '{property}'"),
        )
        .with_resource(ResourceKind::Property, property.to_string())
        .with_recovery(Recovery::CorrectRequest)
    }

    /// The policy engine denied the operation.
    pub fn policy_denied<M: Into<String>>(reason: M) -> Self {
        Self::new(ErrorCode::PolicyDenied, reason).with_recovery(Recovery::Reauthorize)
    }

    /// The backend cannot serve this operation.
    pub fn unsupported<M: Into<String>>(capability: M) -> Self {
        Self::new(ErrorCode::UnsupportedCapability, capability.into())
            .with_resource(ResourceKind::Capability, "")
            .with_recovery(Recovery::Rediscover)
    }

    /// A failure reported by a backend adapter.
    pub fn adapter<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::Adapter, message)
    }

    /// A failure reported by a backend adapter, with the underlying message.
    pub fn adapter_with_source<M: Into<String>, S: Into<String>>(message: M, source: S) -> Self {
        Self::adapter(message).with_source(source)
    }

    /// A temporary backend failure.
    pub fn transient<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::Transient, message).with_recovery(Recovery::Retry)
    }

    /// An internal invariant violation.
    pub fn internal<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::Internal, message).with_recovery(Recovery::Escalate)
    }

    /// The caller is not authenticated.
    pub fn unauthorized<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::Unauthorized, message).with_recovery(Recovery::Reauthorize)
    }

    /// A malformed request.
    pub fn bad_request<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::BadRequest, message).with_recovery(Recovery::CorrectRequest)
    }

    /// A conflict with concurrent state.
    pub fn conflict<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::Conflict, message).with_recovery(Recovery::Reread)
    }

    /// A revision precondition did not match.
    pub fn stale_revision<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::StaleRevision, message)
            .with_side_effect(SideEffectStatus::NotStarted)
            .with_recovery(Recovery::Reread)
    }

    /// A cached plan or tool catalog is stale.
    pub fn stale_schema<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::StaleSchema, message).with_recovery(Recovery::Rediscover)
    }

    /// The deadline elapsed.
    pub fn timeout<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::Timeout, message)
    }

    /// The caller cancelled the operation.
    pub fn cancelled<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::Cancelled, message).with_side_effect(SideEffectStatus::Unknown)
    }

    /// An execution budget was exhausted.
    pub fn budget_exhausted<M: Into<String>>(message: M) -> Self {
        Self::new(ErrorCode::BudgetExhausted, message)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)?;
        if let (Some(kind), Some(resource)) = (self.resource_kind, &self.resource)
            && !resource.is_empty()
        {
            write!(f, " ({kind} '{resource}')")?;
        }
        if let Some(source) = &self.source_msg {
            write!(f, ": {source}")?;
        }
        if self.side_effect == SideEffectStatus::Unknown {
            f.write_str(" [side effects unknown]")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}
