use super::compiler::Compiler;
use super::validation::{ValidationMode, into_validation_result, validate_values};
use super::*;

/// A spec compiled into dense, resolved, executable form.
#[derive(Debug, Clone)]
pub struct CompiledOntology {
    pub(super) spec: Arc<Spec>,
    pub(super) fingerprint: SemanticFingerprint,
    pub(super) generation: u64,
    pub(super) object_types: Vec<CompiledObjectType>,
    pub(super) object_type_index: HashMap<ApiName, ObjectTypeId>,
    pub(super) link_types: Vec<CompiledLinkType>,
    pub(super) link_type_index: HashMap<ApiName, LinkId>,
    pub(super) actions: Vec<CompiledAction>,
    pub(super) action_index: HashMap<ApiName, ActionId>,
    pub(super) traits: Vec<CompiledTrait>,
    pub(super) trait_index: HashMap<ApiName, usize>,
}

impl CompiledOntology {
    /// Compile a spec.
    pub fn compile(spec: &Spec) -> Result<Self, SpecError> {
        Self::compile_shared(Arc::new(spec.clone()))
    }

    /// Compile a spec that is already shared.
    ///
    /// The runtime keeps the `Arc` it was handed rather than deep-copying the
    /// spec, so `Runtime::spec()` is a refcount bump.
    pub fn compile_shared(spec: Arc<Spec>) -> Result<Self, SpecError> {
        Compiler::new(&spec).run()
    }

    /// The spec this ontology was compiled from.
    #[must_use]
    pub fn spec(&self) -> &Arc<Spec> {
        &self.spec
    }

    /// The semantic fingerprint of the compiled spec.
    #[must_use]
    pub fn fingerprint(&self) -> &SemanticFingerprint {
        &self.fingerprint
    }

    /// Monotonic counter of how many times the ontology has been replaced.
    ///
    /// Starts at zero and is bumped by the runtime on each successful apply, so
    /// a caller holding a plan or a tool catalog can tell that the ontology it
    /// was built against is no longer the one in force.
    #[must_use]
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Stamp this ontology with a generation.
    #[must_use]
    pub fn with_generation(mut self, generation: u64) -> Self {
        self.generation = generation;
        self
    }

    /// Every object type, in declaration order.
    #[must_use]
    pub fn object_types(&self) -> &[CompiledObjectType] {
        &self.object_types
    }

    /// Look an object type up by name.
    #[must_use]
    pub fn object_type(&self, name: &str) -> Option<&CompiledObjectType> {
        self.object_type_index
            .get(name)
            .map(|id| &self.object_types[id.index()])
    }

    /// Look an object type up by id.
    #[must_use]
    pub fn object_type_by_id(&self, id: ObjectTypeId) -> Option<&CompiledObjectType> {
        self.object_types.get(id.index())
    }

    /// Every link type, in declaration order.
    #[must_use]
    pub fn link_types(&self) -> &[CompiledLinkType] {
        &self.link_types
    }

    /// Look a link type up by name.
    #[must_use]
    pub fn link_type(&self, name: &str) -> Option<&CompiledLinkType> {
        self.link_type_index
            .get(name)
            .map(|id| &self.link_types[id.index()])
    }

    /// Look a link type up by id.
    #[must_use]
    pub fn link_type_by_id(&self, id: LinkId) -> Option<&CompiledLinkType> {
        self.link_types.get(id.index())
    }

    /// Every action, in declaration order.
    #[must_use]
    pub fn actions(&self) -> &[CompiledAction] {
        &self.actions
    }

    /// Look an action up by name.
    #[must_use]
    pub fn action(&self, name: &str) -> Option<&CompiledAction> {
        self.action_index
            .get(name)
            .map(|id| &self.actions[id.index()])
    }

    /// Look an action up by id.
    #[must_use]
    pub fn action_by_id(&self, id: ActionId) -> Option<&CompiledAction> {
        self.actions.get(id.index())
    }

    /// Every trait, in declaration order.
    #[must_use]
    pub fn traits(&self) -> &[CompiledTrait] {
        &self.traits
    }

    /// Look a trait up by name.
    #[must_use]
    pub fn trait_by_name(&self, name: &str) -> Option<&CompiledTrait> {
        self.trait_index.get(name).map(|index| &self.traits[*index])
    }

    /// Validate a create against an object type.
    ///
    /// Reports every problem it finds, not the first, and fails closed on
    /// anything the object type does not declare — a misspelt property name must
    /// not be silently dropped from the record.
    pub fn validate_create(
        &self,
        object_type: &CompiledObjectType,
        values: &BTreeMap<ApiName, Value>,
    ) -> Result<(), Error> {
        let diagnostics = validate_values(object_type, values, ValidationMode::Create);
        into_validation_result(object_type, diagnostics)
    }

    /// Validate a partial update against an object type.
    ///
    /// Required properties are not checked: a patch states only what it changes.
    /// What it *does* say must be well-formed, and must name real properties.
    pub fn validate_patch(
        &self,
        object_type: &CompiledObjectType,
        values: &BTreeMap<ApiName, Value>,
    ) -> Result<(), Error> {
        let diagnostics = validate_values(object_type, values, ValidationMode::Patch);
        into_validation_result(object_type, diagnostics)
    }
}
