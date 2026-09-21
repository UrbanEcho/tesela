//! Action type definitions.

use serde::{Deserialize, Serialize};
use tesela_core::{ApiName, RiskLevel, Value};

/// A named operation that can be executed against the ontology.
///
/// An action type declares its *contract* — what it may touch, how risky it is,
/// and whether repeating it is safe. It does not carry a handler: the code that
/// runs an action is bound by the host, because a portable ontology cannot ship
/// a function pointer. Those declarations are what the capability layer turns
/// into a tool's side-effect class and approval policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionType {
    /// API name.
    pub api_name: ApiName,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The object type this action operates on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<ApiName>,
    /// JSON schema for the action's input.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<Value>,
    /// JSON schema for the action's output.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<Value>,
    /// How risky the action is to run.
    #[serde(default)]
    pub risk_level: RiskLevel,
    /// What the action may touch.
    #[serde(default)]
    pub effects: ActionEffects,
    /// Semantic object types read by the action.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reads: Vec<ApiName>,
    /// Semantic object types or properties written by the action.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub writes: Vec<ApiName>,
    /// Semantic object types the action may create.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub creates: Vec<ApiName>,
    /// External systems the action may affect.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub external_effects: Vec<ApiName>,
    /// Approval behavior.
    #[serde(default)]
    pub approval: ApprovalPolicy,
    /// Whether callers must supply an expected subject revision.
    #[serde(default)]
    pub requires_revision: bool,
    /// Semantic preconditions checked against current subject state.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub preconditions: Vec<crate::Filter>,
    /// What the host must guarantee about concurrent invocations.
    #[serde(default)]
    pub concurrency: Concurrency,
    /// Template for a caller-supplied idempotency key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// When this action was deprecated, as an ISO-8601 instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deprecated_at: Option<String>,
}

/// Static approval requirement declared by an action contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalPolicy {
    /// Read-only or otherwise safe to execute immediately.
    Never,
    /// Policy or the host decides from the invocation.
    #[default]
    Policy,
    /// Always require explicit host approval.
    Always,
}

/// What an action may touch.
///
/// The default is deliberately pessimistic: an action that declares nothing is
/// assumed to write. Over-declaring a write costs an extra approval prompt;
/// under-declaring one is a security bug, so the ambiguous case resolves in the
/// safe direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionEffects {
    /// The action reads ontology data.
    pub reads: bool,
    /// The action changes ontology data.
    pub writes: bool,
    /// The action calls a system outside Tesela.
    pub external: bool,
    /// Repeating the action with the same input is safe.
    pub idempotent: bool,
}

impl ActionEffects {
    /// The conservative default: reads and writes, no external calls, not
    /// idempotent.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            reads: true,
            writes: true,
            external: false,
            idempotent: false,
        }
    }

    /// Effects for an action that only reads.
    #[must_use]
    pub const fn read_only() -> Self {
        Self {
            reads: true,
            writes: false,
            external: false,
            idempotent: true,
        }
    }

    /// Whether the action can change anything at all.
    #[must_use]
    pub const fn is_read_only(self) -> bool {
        !self.writes && !self.external
    }
}

impl Default for ActionEffects {
    fn default() -> Self {
        Self::new()
    }
}

/// What a host must guarantee about concurrent invocations of an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Concurrency {
    /// The host must not run two invocations for the same subject at once.
    #[default]
    Serial,
    /// Invocations may interleave, and each carries a revision precondition.
    Optimistic,
    /// The host makes no guarantee; the backend's own semantics apply.
    Unrestricted,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn undeclared_effects_are_assumed_to_write() {
        let effects = ActionEffects::default();
        assert!(effects.writes);
        assert!(!effects.is_read_only());
    }

    #[test]
    fn read_only_effects_report_no_mutation() {
        assert!(ActionEffects::read_only().is_read_only());
        assert!(ActionEffects::read_only().idempotent);
    }

    #[test]
    fn concurrency_defaults_to_serial() {
        assert_eq!(Concurrency::default(), Concurrency::Serial);
    }
}
