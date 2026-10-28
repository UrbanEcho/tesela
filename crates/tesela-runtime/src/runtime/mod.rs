//! Compiled, authority-aware ontology execution.

use std::collections::BTreeMap;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use serde::{Deserialize, Serialize};
use tesela_backend::{
    ActionExecutor, ActionRequest, AggregateQuery, Backend, Bindings, ExecutionEvent,
    ExecutionObserver, LinkResolver, Mutation, PolicyDecision, PolicyEngine, PolicyRequest, Query,
    TraversalQuery, check_backend, compare_records,
};
use tesela_core::{
    ApiName, Error, ErrorCode, ExecutionContext, Key, Operation, ResourceKind, SideEffectStatus,
    Value,
};
use tesela_ir::{
    ActionResult, AggregateResult, Filter, MutationResult, Page, Record, SemanticFingerprint, Spec,
};

use crate::capability::{Capability, CapabilityCatalog, CapabilityInvocation, ToolExposure};
use crate::compile::{CompiledObjectType, CompiledOntology, CompiledQuery, compile_filter};

/// A machine-readable explanation of an operation before execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    /// Ontology generation used to compile the plan.
    pub generation: u64,
    /// Semantic operation.
    pub operation: Operation,
    /// Resource name.
    pub resource: ApiName,
    /// Backend diagnostic name.
    pub backend: String,
    /// Effective filter after policy contribution.
    pub filter: Option<Filter>,
    /// Deterministic sort fields; primary-key tie-breaking is implicit.
    pub sort: Vec<tesela_backend::Sort>,
    /// Effective bounded result limit.
    pub limit: usize,
    /// Whether filtering executes in the backend.
    pub filter_pushdown: bool,
    /// Whether aggregation executes in the backend.
    pub aggregate_pushdown: bool,
}

/// Policy outcome produced by the same authorization path used for execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccessExplanation {
    /// Whether execution is allowed.
    pub allowed: bool,
    /// Optional denial rationale.
    pub reason: Option<String>,
    /// Row predicate contributed by policy.
    pub row_filter: Option<Filter>,
    /// Fields removed from the result.
    pub redactions: Vec<ApiName>,
    /// Whether human approval is required.
    pub approval_required: bool,
    /// Additional semantic input predicates.
    pub input_constraints: Vec<Filter>,
    /// Host-requested field replacements.
    pub transformations: BTreeMap<ApiName, Value>,
    /// Machine-readable obligations.
    pub obligations: BTreeMap<String, Value>,
}

/// Provenance emitted for every successful operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionReceipt {
    /// Request id supplied by the host.
    pub request_id: String,
    /// Ontology generation.
    pub generation: u64,
    /// Semantic fingerprint.
    pub ontology: SemanticFingerprint,
    /// Principal id.
    pub principal: String,
    /// Operation.
    pub operation: Operation,
    /// Resource.
    pub resource: ApiName,
    /// Backend source.
    pub source: String,
    /// Applied redactions.
    pub redactions: Vec<ApiName>,
    /// Declared semantic effects for this call.
    pub effects: tesela_ir::ActionEffects,
    /// Whether approval was required by policy or contract.
    pub approval_required: bool,
    /// Whether the execution context carried that approval.
    pub approval_granted: bool,
    /// Policy obligations.
    pub obligations: BTreeMap<String, Value>,
    /// Elapsed microseconds.
    pub elapsed_micros: u64,
}

/// Successful value and its execution provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Execution<T> {
    /// Operation output.
    pub output: T,
    /// Provenance receipt.
    pub receipt: ExecutionReceipt,
}

/// Builder for an immutable runtime configuration.
pub struct RuntimeBuilder {
    spec: Spec,
    bindings: Bindings,
    policy: Arc<dyn PolicyEngine>,
    observers: Vec<Arc<dyn ExecutionObserver>>,
    max_query_limit: usize,
}

impl RuntimeBuilder {
    /// Begin construction with the required semantic and host contracts.
    #[must_use]
    pub fn new(spec: Spec, bindings: Bindings, policy: Arc<dyn PolicyEngine>) -> Self {
        Self {
            spec,
            bindings,
            policy,
            observers: Vec::new(),
            max_query_limit: 1_000,
        }
    }
    /// Set the hard row limit applied even when a query omits one.
    #[must_use]
    pub fn max_query_limit(mut self, limit: usize) -> Self {
        self.max_query_limit = limit;
        self
    }
    /// Add an infallible observer.
    #[must_use]
    pub fn observer(mut self, observer: Arc<dyn ExecutionObserver>) -> Self {
        self.observers.push(observer);
        self
    }
    /// Compile, bind, and freeze the runtime.
    pub fn build(self) -> Result<Arc<Runtime>, Error> {
        if self.max_query_limit == 0 {
            return Err(Error::validation("max_query_limit must be positive"));
        }
        let snapshot = ExecutionSnapshot::compile(self.spec, &self.bindings, 1)?;
        Ok(Arc::new(Runtime {
            snapshot: ArcSwap::from_pointee(snapshot),
            bindings: self.bindings,
            policy: self.policy,
            observers: self.observers.into(),
            max_query_limit: self.max_query_limit,
        }))
    }
}

struct ExecutionSnapshot {
    ontology: Arc<CompiledOntology>,
    objects: Box<[Option<Arc<dyn Backend>>]>,
    links: Box<[Option<Arc<dyn LinkResolver>>]>,
    actions: Box<[Option<Arc<dyn ActionExecutor>>]>,
}

impl ExecutionSnapshot {
    fn compile(spec: Spec, bindings: &Bindings, generation: u64) -> Result<Self, Error> {
        let ontology =
            Arc::new(CompiledOntology::compile_shared(spec.into())?.with_generation(generation));
        for name in bindings.object_names() {
            if ontology.object_type(name).is_none() {
                return Err(Error::validation(format!(
                    "object binding '{name}' is not declared by the spec"
                )));
            }
        }
        for name in bindings.link_names() {
            if ontology.link_type(name).is_none() {
                return Err(Error::validation(format!(
                    "link binding '{name}' is not declared by the spec"
                )));
            }
        }
        for name in bindings.action_names() {
            if ontology.action(name).is_none() {
                return Err(Error::validation(format!(
                    "action binding '{name}' is not declared by the spec"
                )));
            }
        }
        let mut objects = Vec::with_capacity(ontology.object_types().len());
        for object in ontology.object_types() {
            let backend = bindings.object(object.api_name()).cloned();
            if let Some(backend) = &backend {
                check_backend(backend.as_ref())?;
            }
            objects.push(backend);
        }
        let links = ontology
            .link_types()
            .iter()
            .map(|link| bindings.link(link.api_name()).cloned())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let actions = ontology
            .actions()
            .iter()
            .map(|action| bindings.action(action.api_name()).cloned())
            .collect::<Vec<_>>()
            .into_boxed_slice();
        Ok(Self {
            ontology,
            objects: objects.into_boxed_slice(),
            links,
            actions,
        })
    }
}

/// Lock-free read-mostly semantic runtime.
pub struct Runtime {
    snapshot: ArcSwap<ExecutionSnapshot>,
    bindings: Bindings,
    policy: Arc<dyn PolicyEngine>,
    observers: Arc<[Arc<dyn ExecutionObserver>]>,
    max_query_limit: usize,
}

impl Runtime {
    /// Build a runtime.
    #[must_use]
    pub fn builder(
        spec: Spec,
        bindings: Bindings,
        policy: Arc<dyn PolicyEngine>,
    ) -> RuntimeBuilder {
        RuntimeBuilder::new(spec, bindings, policy)
    }
    /// Current semantic spec.
    #[must_use]
    pub fn spec(&self) -> Arc<Spec> {
        Arc::clone(self.snapshot.load().ontology.spec())
    }
    /// Current compiled ontology.
    #[must_use]
    pub fn ontology(&self) -> Arc<CompiledOntology> {
        Arc::clone(&self.snapshot.load().ontology)
    }
    /// Compile and bind before atomically publishing a replacement.
    pub fn apply_spec(&self, spec: Spec) -> Result<(), Error> {
        let generation = self
            .snapshot
            .load()
            .ontology
            .generation()
            .checked_add(1)
            .ok_or_else(|| Error::internal("ontology generation overflow"))?;
        self.snapshot.store(Arc::new(ExecutionSnapshot::compile(
            spec,
            &self.bindings,
            generation,
        )?));
        Ok(())
    }
}

mod access;
mod actions;
mod aggregate;
mod auth;
mod data;
mod project;
mod schema;
mod support;

use support::*;
