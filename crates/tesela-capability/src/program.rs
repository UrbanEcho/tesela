//! Straight-line capability programs: several calls validated together and run in order.

use serde::{Deserialize, Serialize};
use tesela_core::Value;
use tesela_ir::{Revision, SemanticFingerprint};

/// Most steps a program may contain.
pub const MAX_PROGRAM_STEPS: usize = 16;

/// One call of a program.
///
/// Input may reference the result of an earlier step with
/// `{"$ref": "<step id>", "field": "<name>"}`; the reference expands to that
/// field's values (the primary keys when `field` is omitted).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProgramStep {
    /// Identifier later steps use to reference this step's result.
    pub id: String,
    /// Selected capability, by qualified name.
    pub capability: String,
    /// Input payload, shaped by the capability's `input_schema`.
    #[serde(default)]
    pub input: Value,
    /// Subject revision the caller expects; required when the capability requires one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<Revision>,
}

/// A typed straight-line program over catalog capabilities.
///
/// There is no branching or looping. The whole program is checked against the
/// catalog, the caller's approvals and the remaining budget before its first
/// step runs, and the last step's result is the program's result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Program {
    /// Catalog generation presented to the caller.
    pub generation: u64,
    /// Catalog fingerprint presented to the caller.
    pub fingerprint: SemanticFingerprint,
    /// Steps, run in order.
    pub steps: Vec<ProgramStep>,
    /// Keep the last step's records in a handle instead of returning them inline.
    #[serde(default)]
    pub retain_results: bool,
}
