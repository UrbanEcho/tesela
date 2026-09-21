//! Closure adapters, immutable bindings, policy, and observation.

use super::*;

/// Small closure-based backend for application repositories and services.
pub struct BackendBuilder {
    name: Arc<str>,
    search: Option<Arc<SearchFn>>,
    get: Option<Arc<GetFn>>,
    create: Option<Arc<CreateFn>>,
    update: Option<Arc<UpdateFn>>,
    delete: Option<Arc<DeleteFn>>,
    upsert: Option<Arc<UpsertFn>>,
    batch: Option<Arc<BatchFn>>,
    aggregate: Option<Arc<AggregateFn>>,
}
impl BackendBuilder {
    /// Start with no capabilities.
    #[must_use]
    pub fn new(name: impl AsRef<str>) -> Self {
        Self {
            name: Arc::from(name.as_ref()),
            search: None,
            get: None,
            create: None,
            update: None,
            delete: None,
            upsert: None,
            batch: None,
            aggregate: None,
        }
    }
    /// Register search.
    #[must_use]
    pub fn search(
        mut self,
        handler: impl Fn(&ApiName, &Query) -> Result<Page, Error> + Send + Sync + 'static,
    ) -> Self {
        self.search = Some(Arc::new(handler));
        self
    }
    /// Register get.
    #[must_use]
    pub fn get(
        mut self,
        handler: impl Fn(&ApiName, &Key) -> Result<Option<Record>, Error> + Send + Sync + 'static,
    ) -> Self {
        self.get = Some(Arc::new(handler));
        self
    }
    /// Register create.
    #[must_use]
    pub fn create(
        mut self,
        handler: impl Fn(&ApiName, Key, BTreeMap<ApiName, Value>) -> Result<MutationResult, Error>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.create = Some(Arc::new(handler));
        self
    }
    /// Register conditional update.
    #[must_use]
    pub fn update(
        mut self,
        handler: impl Fn(
            &ApiName,
            &Key,
            BTreeMap<ApiName, Value>,
            Option<&Revision>,
        ) -> Result<MutationResult, Error>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.update = Some(Arc::new(handler));
        self
    }
    /// Register conditional delete.
    #[must_use]
    pub fn delete(
        mut self,
        handler: impl Fn(&ApiName, &Key, Option<&Revision>) -> Result<MutationResult, Error>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.delete = Some(Arc::new(handler));
        self
    }
    /// Register native upsert.
    #[must_use]
    pub fn upsert(
        mut self,
        handler: impl Fn(
            &ApiName,
            Key,
            BTreeMap<ApiName, Value>,
            Option<&Revision>,
        ) -> Result<MutationResult, Error>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.upsert = Some(Arc::new(handler));
        self
    }
    /// Register native atomic batch.
    #[must_use]
    pub fn batch(
        mut self,
        handler: impl Fn(&ApiName, &ApiName, &[Mutation]) -> Result<MutationResult, Error>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.batch = Some(Arc::new(handler));
        self
    }
    /// Register aggregate pushdown.
    #[must_use]
    pub fn aggregate(
        mut self,
        handler: impl Fn(&ApiName, &AggregateQuery) -> Result<AggregateResult, Error>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.aggregate = Some(Arc::new(handler));
        self
    }
    /// Freeze the backend.
    #[must_use]
    pub fn build(self) -> Arc<Self> {
        Arc::new(self)
    }
}
impl Backend for BackendBuilder {
    fn backend_type(&self) -> &str {
        &self.name
    }
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            search: self.search.is_some(),
            get: self.get.is_some(),
            create: self.create.is_some(),
            update: self.update.is_some(),
            delete: self.delete.is_some(),
            aggregate: self.aggregate.is_some(),
            upsert: self.upsert.is_some(),
            batch: self.batch.is_some(),
            conditional_mutation: self.update.is_some()
                || self.delete.is_some()
                || self.upsert.is_some(),
        }
    }
    fn search(&self, object: &ApiName, query: &Query) -> Result<Page, Error> {
        self.search
            .as_ref()
            .ok_or_else(|| Error::unsupported("search"))?(object, query)
    }
    fn get(&self, object: &ApiName, key: &Key) -> Result<Option<Record>, Error> {
        self.get.as_ref().ok_or_else(|| Error::unsupported("get"))?(object, key)
    }
    fn create(
        &self,
        object: &ApiName,
        key: Key,
        values: BTreeMap<ApiName, Value>,
    ) -> Result<MutationResult, Error> {
        self.create
            .as_ref()
            .ok_or_else(|| Error::unsupported("create"))?(object, key, values)
    }
    fn update(
        &self,
        object: &ApiName,
        key: &Key,
        values: BTreeMap<ApiName, Value>,
        expected: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        self.update
            .as_ref()
            .ok_or_else(|| Error::unsupported("update"))?(object, key, values, expected)
    }
    fn delete(
        &self,
        object: &ApiName,
        key: &Key,
        expected: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        self.delete
            .as_ref()
            .ok_or_else(|| Error::unsupported("delete"))?(object, key, expected)
    }
    fn upsert(
        &self,
        object: &ApiName,
        key: Key,
        values: BTreeMap<ApiName, Value>,
        expected: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        self.upsert
            .as_ref()
            .ok_or_else(|| Error::unsupported("atomic upsert"))?(
            object, key, values, expected
        )
    }
    fn batch(
        &self,
        object: &ApiName,
        primary_key: &ApiName,
        mutations: &[Mutation],
    ) -> Result<MutationResult, Error> {
        self.batch
            .as_ref()
            .ok_or_else(|| Error::unsupported("atomic batch"))?(
            object, primary_key, mutations
        )
    }
    fn aggregate(
        &self,
        object: &ApiName,
        query: &AggregateQuery,
    ) -> Result<AggregateResult, Error> {
        self.aggregate
            .as_ref()
            .ok_or_else(|| Error::unsupported("aggregate"))?(object, query)
    }
}

/// Immutable host wiring.
#[derive(Clone, Default)]
pub struct Bindings {
    objects: Arc<HashMap<ApiName, Arc<dyn Backend>>>,
    links: Arc<HashMap<ApiName, Arc<dyn LinkResolver>>>,
    actions: Arc<HashMap<ApiName, Arc<dyn ActionExecutor>>>,
}
impl Bindings {
    /// Object backend.
    #[must_use]
    pub fn object(&self, name: &ApiName) -> Option<&Arc<dyn Backend>> {
        self.objects.get(name)
    }
    /// Link resolver.
    #[must_use]
    pub fn link(&self, name: &ApiName) -> Option<&Arc<dyn LinkResolver>> {
        self.links.get(name)
    }
    /// Action executor.
    #[must_use]
    pub fn action(&self, name: &ApiName) -> Option<&Arc<dyn ActionExecutor>> {
        self.actions.get(name)
    }
    /// Bound object names, for construction-time semantic validation.
    pub fn object_names(&self) -> impl Iterator<Item = &ApiName> {
        self.objects.keys()
    }
    /// Bound link names, for construction-time semantic validation.
    pub fn link_names(&self) -> impl Iterator<Item = &ApiName> {
        self.links.keys()
    }
    /// Bound action names, for construction-time semantic validation.
    pub fn action_names(&self) -> impl Iterator<Item = &ApiName> {
        self.actions.keys()
    }
}

/// Startup builder for immutable bindings.
#[derive(Default)]
pub struct BindingsBuilder {
    objects: HashMap<ApiName, Arc<dyn Backend>>,
    links: HashMap<ApiName, Arc<dyn LinkResolver>>,
    actions: HashMap<ApiName, Arc<dyn ActionExecutor>>,
}
impl BindingsBuilder {
    /// Start empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    /// Bind an object.
    pub fn bind_object(&mut self, name: ApiName, backend: Arc<dyn Backend>) -> &mut Self {
        self.objects.insert(name, backend);
        self
    }
    /// Bind a link.
    pub fn bind_link(&mut self, name: ApiName, resolver: Arc<dyn LinkResolver>) -> &mut Self {
        self.links.insert(name, resolver);
        self
    }
    /// Bind an action.
    pub fn bind_action(&mut self, name: ApiName, executor: Arc<dyn ActionExecutor>) -> &mut Self {
        self.actions.insert(name, executor);
        self
    }
    /// Freeze bindings.
    #[must_use]
    pub fn build(self) -> Bindings {
        Bindings {
            objects: Arc::new(self.objects),
            links: Arc::new(self.links),
            actions: Arc::new(self.actions),
        }
    }
}

/// Evaluates host policy.
pub trait PolicyEngine: Send + Sync {
    /// Evaluate one borrowed request.
    fn evaluate(&self, request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error>;
}

/// Denies every request.
pub struct DenyAllPolicy;
impl PolicyEngine for DenyAllPolicy {
    fn evaluate(&self, request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error> {
        Ok(PolicyDecision {
            allow: false,
            reason: Some(format!(
                "no policy allowed {:?} on {}",
                request.operation, request.resource
            )),
            ..PolicyDecision::default()
        })
    }
}

/// Machine-readable lifecycle record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionEvent {
    /// Request id.
    pub request_id: Arc<str>,
    /// Operation name.
    pub operation: Arc<str>,
    /// Resource.
    pub resource: ApiName,
    /// Success flag.
    pub success: bool,
    /// Side-effect state.
    pub side_effect: tesela_core::SideEffectStatus,
}

/// Infallible observer; durable audit belongs in the host transaction/outbox.
pub trait ExecutionObserver: Send + Sync {
    /// Observe one event. Implementations must not panic.
    fn observe(&self, event: &ExecutionEvent);
}

/// Check internal consistency of advertised capabilities.
pub fn check_backend(backend: &dyn Backend) -> Result<(), Error> {
    let capabilities = backend.capabilities();
    if capabilities.conditional_mutation
        && !(capabilities.update || capabilities.delete || capabilities.upsert)
    {
        return Err(Error::validation(
            "conditional mutation has no mutation operation",
        ));
    }
    Ok(())
}
