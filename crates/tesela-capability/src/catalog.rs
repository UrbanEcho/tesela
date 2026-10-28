//! Versioned capability catalogs and invocation identity.

use serde::{Deserialize, Serialize};
use tesela_core::{Operation, RiskLevel, Value};
use tesela_ir::{ActionEffects, Concurrency, Filter, SemanticFingerprint};

/// How a capability may be exposed to a model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolExposure {
    /// Included in the initial tool surface.
    Direct,
    /// Returned through deferred search.
    Discoverable,
    /// Available only to a future compiled program surface.
    ProgramOnly,
    /// Not callable by the principal.
    Hidden,
}

/// One ontology-derived operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capability {
    /// Stable qualified name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Semantic operation executed by this capability.
    pub operation: Operation,
    /// Exposure selected by the compiler.
    pub exposure: ToolExposure,
    /// JSON-schema-shaped input contract.
    pub input_schema: Value,
    /// Declared action effects; read capabilities use `read_only`.
    pub effects: ActionEffects,
    /// Explicit approval requirement.
    pub approval_required: bool,
    /// Static action risk.
    pub risk: RiskLevel,
    /// Whether repeating the same invocation is declared safe.
    pub idempotent: bool,
    /// Host concurrency contract for actions.
    pub concurrency: Option<Concurrency>,
    /// Whether a subject revision must accompany the call.
    pub requires_revision: bool,
    /// Semantic predicates checked immediately before execution.
    pub preconditions: Vec<Filter>,
}

/// Versioned capability surface compiled for one principal and task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityCatalog {
    /// Ontology generation.
    pub generation: u64,
    /// Semantic identity.
    pub fingerprint: SemanticFingerprint,
    /// Projected operations.
    pub capabilities: Vec<Capability>,
}

impl CapabilityCatalog {
    /// Search discoverable capabilities. Exact and qualified names always rank first.
    #[must_use]
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Capability> {
        let query = query.to_ascii_lowercase();
        let terms: Vec<&str> = query
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .filter(|term| !term.is_empty())
            .collect();
        let mut scored: Vec<_> = self
            .capabilities
            .iter()
            .filter(|capability| capability.exposure != ToolExposure::Hidden)
            .map(|capability| {
                let name = capability.name.to_ascii_lowercase();
                let text = format!("{name} {}", capability.description.to_ascii_lowercase());
                let exact =
                    usize::from(name == query || name.rsplit('.').next() == Some(query.as_str()));
                let lexical: usize = terms.iter().map(|term| text.matches(term).count()).sum();
                (exact, lexical, capability)
            })
            .filter(|(exact, lexical, _)| *exact > 0 || *lexical > 0)
            .collect();
        scored.sort_by(|left, right| {
            right
                .0
                .cmp(&left.0)
                .then_with(|| right.1.cmp(&left.1))
                .then_with(|| left.2.name.cmp(&right.2.name))
        });
        scored
            .into_iter()
            .take(limit)
            .map(|(_, _, capability)| capability)
            .collect()
    }
}

/// Invocation identity used to reject stale calls.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInvocation {
    /// Catalog generation presented to the caller.
    pub generation: u64,
    /// Catalog fingerprint presented to the caller.
    pub fingerprint: SemanticFingerprint,
    /// Selected capability.
    pub capability: String,
    /// Input payload.
    pub input: Value,
}
