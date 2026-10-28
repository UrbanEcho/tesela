//! Versioned capability catalogs, their projection vocabulary, and invocation identity.

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

/// Behavioral hints a host may use to decide how to present a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityAnnotations {
    /// The operation reads without writing or calling external systems.
    pub read_only: bool,
    /// The operation writes and carries high risk.
    pub destructive: bool,
    /// Repeating the same invocation is declared safe.
    pub idempotent: bool,
    /// The operation reaches systems outside the ontology's own data.
    pub open_world: bool,
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
    /// Behavioral hints derived from effects, risk, and idempotency.
    pub annotations: CapabilityAnnotations,
    /// Explicit approval requirement.
    pub approval_required: bool,
    /// Static action risk.
    pub risk: RiskLevel,
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

/// Split free text into lowercase ASCII alphanumeric tokens.
#[must_use]
pub fn tokenize(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_ascii_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// Whether `phrase` appears as a contiguous run of whole tokens inside `tokens`.
#[must_use]
pub fn contains_phrase(tokens: &[String], phrase: &[String]) -> bool {
    !phrase.is_empty() && tokens.windows(phrase.len()).any(|window| window == phrase)
}

impl CapabilityCatalog {
    /// Search discoverable capabilities. Exact and qualified names always rank first.
    ///
    /// Matching is by whole token, so `cust` does not match `customer`.
    #[must_use]
    pub fn search(&self, query: &str, limit: usize) -> Vec<&Capability> {
        let query_name = query.to_ascii_lowercase();
        let terms = tokenize(query);
        let mut scored: Vec<_> = self
            .capabilities
            .iter()
            .filter(|capability| capability.exposure != ToolExposure::Hidden)
            .map(|capability| {
                let name = capability.name.to_ascii_lowercase();
                let exact = usize::from(
                    name == query_name || name.rsplit('.').next() == Some(query_name.as_str()),
                );
                let mut tokens = tokenize(&capability.name);
                tokens.extend(tokenize(&capability.description));
                let lexical = terms.iter().filter(|term| tokens.contains(term)).count();
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

/// One call against a capability, carrying the catalog identity it was issued under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityInvocation {
    /// Catalog generation presented to the caller.
    pub generation: u64,
    /// Catalog fingerprint presented to the caller.
    pub fingerprint: SemanticFingerprint,
    /// Selected capability, by qualified name.
    pub capability: String,
    /// Input payload, shaped by the capability's `input_schema`.
    #[serde(default)]
    pub input: Value,
}
