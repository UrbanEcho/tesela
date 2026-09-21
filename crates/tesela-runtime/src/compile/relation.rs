use super::*;

/// A link type with its endpoints resolved to object type ids.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledLinkType {
    pub(super) id: LinkId,
    pub(super) api_name: ApiName,
    pub(super) display: Option<String>,
    pub(super) from: ObjectTypeId,
    pub(super) to: ObjectTypeId,
    pub(super) cardinality: LinkCardinality,
    pub(super) inverse: Option<LinkId>,
    pub(super) deprecated_at: Option<String>,
}

impl CompiledLinkType {
    /// Dense id within the owning ontology.
    #[must_use]
    pub fn id(&self) -> LinkId {
        self.id
    }

    /// API name.
    #[must_use]
    pub fn api_name(&self) -> &ApiName {
        &self.api_name
    }

    /// Display name.
    #[must_use]
    pub fn display(&self) -> Option<&str> {
        self.display.as_deref()
    }

    /// The id of the source object type.
    #[must_use]
    pub fn from(&self) -> ObjectTypeId {
        self.from
    }

    /// The id of the target object type.
    #[must_use]
    pub fn to(&self) -> ObjectTypeId {
        self.to
    }

    /// Declared cardinality.
    #[must_use]
    pub fn cardinality(&self) -> LinkCardinality {
        self.cardinality
    }

    /// The link that reads in the opposite direction, when one is declared.
    #[must_use]
    pub fn inverse(&self) -> Option<LinkId> {
        self.inverse
    }

    /// When the link type was deprecated, if it was.
    #[must_use]
    pub fn deprecated_at(&self) -> Option<&str> {
        self.deprecated_at.as_deref()
    }
}

impl std::fmt::Display for CompiledLinkType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.api_name)
    }
}

/// An action type with its subject resolved and its contract preserved.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledAction {
    pub(super) id: ActionId,
    pub(super) api_name: ApiName,
    pub(super) display: Option<String>,
    pub(super) description: Option<String>,
    pub(super) subject: Option<ObjectTypeId>,
    pub(super) input_schema: Option<Value>,
    pub(super) output_schema: Option<Value>,
    pub(super) risk_level: RiskLevel,
    pub(super) effects: ActionEffects,
    pub(super) reads: Vec<ApiName>,
    pub(super) writes: Vec<ApiName>,
    pub(super) creates: Vec<ApiName>,
    pub(super) external_effects: Vec<ApiName>,
    pub(super) approval: ApprovalPolicy,
    pub(super) requires_revision: bool,
    pub(super) preconditions: Vec<Filter>,
    pub(super) concurrency: Concurrency,
    pub(super) idempotency_key: Option<String>,
    pub(super) deprecated_at: Option<String>,
}

impl CompiledAction {
    /// Dense id within the owning ontology.
    #[must_use]
    pub fn id(&self) -> ActionId {
        self.id
    }

    /// API name.
    #[must_use]
    pub fn api_name(&self) -> &ApiName {
        &self.api_name
    }

    /// Display name.
    #[must_use]
    pub fn display(&self) -> Option<&str> {
        self.display.as_deref()
    }

    /// Description.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The object type the action operates on, when it declares one.
    #[must_use]
    pub fn subject(&self) -> Option<ObjectTypeId> {
        self.subject
    }

    /// JSON schema for the action's input.
    #[must_use]
    pub fn input_schema(&self) -> Option<&Value> {
        self.input_schema.as_ref()
    }

    /// JSON schema for the action's output.
    #[must_use]
    pub fn output_schema(&self) -> Option<&Value> {
        self.output_schema.as_ref()
    }

    /// How risky the action is to run.
    #[must_use]
    pub fn risk_level(&self) -> RiskLevel {
        self.risk_level
    }

    /// What the action may touch.
    #[must_use]
    pub fn effects(&self) -> ActionEffects {
        self.effects
    }

    /// Semantic read targets.
    #[must_use]
    pub fn reads(&self) -> &[ApiName] {
        &self.reads
    }
    /// Semantic write targets.
    #[must_use]
    pub fn writes(&self) -> &[ApiName] {
        &self.writes
    }
    /// Semantic create targets.
    #[must_use]
    pub fn creates(&self) -> &[ApiName] {
        &self.creates
    }
    /// External effect targets.
    #[must_use]
    pub fn external_effects(&self) -> &[ApiName] {
        &self.external_effects
    }
    /// Static approval policy.
    #[must_use]
    pub fn approval(&self) -> ApprovalPolicy {
        self.approval
    }
    /// Whether callers must supply an expected revision.
    #[must_use]
    pub fn requires_revision(&self) -> bool {
        self.requires_revision
    }
    /// Declared state predicates.
    #[must_use]
    pub fn preconditions(&self) -> &[Filter] {
        &self.preconditions
    }

    /// What the host must guarantee about concurrent invocations.
    #[must_use]
    pub fn concurrency(&self) -> Concurrency {
        self.concurrency
    }

    /// Template for a caller-supplied idempotency key.
    #[must_use]
    pub fn idempotency_key(&self) -> Option<&str> {
        self.idempotency_key.as_deref()
    }

    /// When the action was deprecated, if it was.
    #[must_use]
    pub fn deprecated_at(&self) -> Option<&str> {
        self.deprecated_at.as_deref()
    }

    /// Whether running the action can change anything at all.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        self.effects.is_read_only()
    }
}

impl std::fmt::Display for CompiledAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.api_name)
    }
}

/// A trait as declared, retained for diagnostics and documentation.
///
/// Traits themselves are not executable: they exist to contribute properties,
/// which flattening has already done. Keeping them means a host can still say
/// "which traits does this object type implement" without re-reading the spec.
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledTrait {
    pub(super) api_name: ApiName,
    pub(super) display: Option<String>,
    pub(super) description: Option<String>,
    pub(super) properties: Vec<CompiledProperty>,
}

impl CompiledTrait {
    /// API name.
    #[must_use]
    pub fn api_name(&self) -> &ApiName {
        &self.api_name
    }

    /// Display name.
    #[must_use]
    pub fn display(&self) -> Option<&str> {
        self.display.as_deref()
    }

    /// Description.
    #[must_use]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The properties the trait contributes.
    #[must_use]
    pub fn properties(&self) -> &[CompiledProperty] {
        &self.properties
    }
}
