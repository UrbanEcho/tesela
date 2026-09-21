use std::fmt;

use serde::{Deserialize, Serialize};

/// Machine-readable error classification.
///
/// Codes are part of the public contract: they are stable across releases and
/// are what callers should branch on. New codes may be added, so match
/// exhaustively only with a trailing wildcard.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// The request or spec is structurally invalid.
    Validation,
    /// The request is well-formed but not acceptable in the current state.
    BadRequest,
    /// An object type named by the request does not exist.
    UnknownObjectType,
    /// A property named by the request does not exist on the object type.
    UnknownProperty,
    /// A link type named by the request does not exist.
    UnknownLink,
    /// An action named by the request does not exist.
    UnknownAction,
    /// The requested resource does not exist.
    NotFound,
    /// The write conflicts with concurrent state.
    Conflict,
    /// A revision precondition did not match the stored revision.
    StaleRevision,
    /// A non-revision precondition did not hold.
    PreconditionFailed,
    /// A compiled plan or capability catalog targets an obsolete ontology.
    StaleSchema,
    /// The caller is not authenticated.
    Unauthorized,
    /// The policy engine denied the operation.
    PolicyDenied,
    /// The operation requires approval that has not been granted.
    ApprovalRequired,
    /// The backend cannot serve this operation.
    UnsupportedCapability,
    /// The capability exists but is currently unavailable.
    Unavailable,
    /// An execution budget (rows, backend calls, bytes) was exhausted.
    BudgetExhausted,
    /// The deadline elapsed.
    Timeout,
    /// The caller cancelled the operation.
    Cancelled,
    /// A backend failure that is expected to be temporary.
    Transient,
    /// A backend failure reported by an adapter.
    Adapter,
    /// An invariant was violated inside Tesela.
    Internal,
}

impl ErrorCode {
    /// The stable string form of this code.
    ///
    /// This is the value that crosses the Python, JSON and tool boundaries.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Validation => "validation",
            Self::BadRequest => "bad_request",
            Self::UnknownObjectType => "unknown_object_type",
            Self::UnknownProperty => "unknown_property",
            Self::UnknownLink => "unknown_link",
            Self::UnknownAction => "unknown_action",
            Self::NotFound => "not_found",
            Self::Conflict => "conflict",
            Self::StaleRevision => "stale_revision",
            Self::PreconditionFailed => "precondition_failed",
            Self::StaleSchema => "stale_schema",
            Self::Unauthorized => "unauthorized",
            Self::PolicyDenied => "policy_denied",
            Self::ApprovalRequired => "approval_required",
            Self::UnsupportedCapability => "unsupported_capability",
            Self::Unavailable => "unavailable",
            Self::BudgetExhausted => "budget_exhausted",
            Self::Timeout => "timeout",
            Self::Cancelled => "cancelled",
            Self::Transient => "transient",
            Self::Adapter => "adapter",
            Self::Internal => "internal",
        }
    }

    /// The coarse category this code belongs to.
    #[must_use]
    pub const fn category(self) -> ErrorCategory {
        match self {
            Self::Validation
            | Self::BadRequest
            | Self::UnknownObjectType
            | Self::UnknownProperty
            | Self::UnknownLink
            | Self::UnknownAction => ErrorCategory::Invalid,
            Self::NotFound => ErrorCategory::NotFound,
            Self::Conflict | Self::StaleRevision | Self::PreconditionFailed | Self::StaleSchema => {
                ErrorCategory::Conflict
            }
            Self::Unauthorized | Self::PolicyDenied | Self::ApprovalRequired => {
                ErrorCategory::Denied
            }
            Self::UnsupportedCapability | Self::Unavailable => ErrorCategory::Capability,
            Self::BudgetExhausted | Self::Timeout | Self::Cancelled => ErrorCategory::Budget,
            Self::Transient | Self::Adapter => ErrorCategory::Backend,
            Self::Internal => ErrorCategory::Internal,
        }
    }

    /// Whether retrying the identical request could plausibly succeed.
    #[must_use]
    pub const fn is_retryable(self) -> bool {
        matches!(
            self.category(),
            ErrorCategory::Backend | ErrorCategory::Conflict
        ) || matches!(self, Self::Unavailable | Self::Timeout)
    }
}

/// Coarse grouping of [`ErrorCode`]s for callers that do not want to match
/// every code.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCategory {
    /// The request or the spec is wrong. Retrying unchanged will not help.
    Invalid,
    /// The resource does not exist.
    NotFound,
    /// Concurrent state changed underneath the operation.
    Conflict,
    /// The caller is not permitted.
    Denied,
    /// The backend cannot do this, now or at all.
    Capability,
    /// A budget, deadline or cancellation limit was reached.
    Budget,
    /// A backend failure.
    Backend,
    /// A Tesela invariant was violated.
    Internal,
}

/// Whether a failed operation changed stored state.
///
/// This is the field that makes retry decisions safe. An `Unknown` status means
/// the caller must read back before retrying a write, because the write may have
/// been applied.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectStatus {
    /// The operation cannot have written anything.
    #[default]
    None,
    /// A write was requested but provably never began.
    NotStarted,
    /// The operation may or may not have been applied.
    Unknown,
    /// The write completed and is durable.
    Completed,
}

impl SideEffectStatus {
    /// Whether a retry of the same request is safe with respect to prior state.
    ///
    /// Only meaningful in combination with [`ErrorCode::is_retryable`].
    #[must_use]
    pub const fn allows_retry(self) -> bool {
        matches!(self, Self::None | Self::NotStarted)
    }

    /// The stable string form of this status.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::NotStarted => "not_started",
            Self::Unknown => "unknown",
            Self::Completed => "completed",
        }
    }
}

/// The action most likely to succeed after a failure.
///
/// This is advisory guidance for an automated caller, not an instruction the
/// runtime enforces.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Recovery {
    /// Obtain fresh credentials or a new grant, then retry.
    Reauthorize,
    /// Re-read the resource to learn its current revision, then retry.
    Reread,
    /// Re-discover the capability or tool catalog; it has changed.
    Rediscover,
    /// The request itself is wrong; correct it before retrying.
    CorrectRequest,
    /// Retry the same request unchanged.
    Retry,
    /// The host must resolve this; the runtime cannot.
    Escalate,
}

impl Recovery {
    /// Stable machine-readable recovery name.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Reauthorize => "reauthorize",
            Self::Reread => "reread",
            Self::Rediscover => "rediscover",
            Self::CorrectRequest => "correct_request",
            Self::Retry => "retry",
            Self::Escalate => "escalate",
        }
    }
}

/// The kind of resource an operation or error refers to.
///
/// Replaces the unvalidated `resource_kind: String` that used to be threaded
/// through policy requests and audit events.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    /// The ontology specification itself.
    Spec,
    /// An object type.
    ObjectType,
    /// A property of an object type.
    Property,
    /// A link type.
    LinkType,
    /// An action type.
    Action,
    /// A record.
    Record,
    /// A backend.
    Backend,
    /// A query or execution plan.
    Query,
    /// A projected tool.
    Tool,
    /// A capability.
    Capability,
}

impl ResourceKind {
    /// The stable string form of this kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Spec => "spec",
            Self::ObjectType => "object_type",
            Self::Property => "property",
            Self::LinkType => "link_type",
            Self::Action => "action",
            Self::Record => "record",
            Self::Backend => "backend",
            Self::Query => "query",
            Self::Tool => "tool",
            Self::Capability => "capability",
        }
    }
}

impl fmt::Display for ResourceKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
