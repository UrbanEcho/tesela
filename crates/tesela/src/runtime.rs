//! Typed runtime construction and execution.

use std::marker::PhantomData;
use std::sync::Arc;

use crate::backend::ObjectBinding;
use crate::error::Error;
use crate::model::{Action, Execution, Link, Object, Schema, encode_key};
use crate::plan::{Plan, Query};
use crate::raw::{
    self, ActionExecutor, ActionRequest, ActionResult, BindingsBuilder, LinkResolver, Spec,
};

/// The only general-purpose stateful object in the public API.
pub struct Runtime {
    pub(crate) inner: Arc<raw::Runtime>,
}

impl Runtime {
    /// Start an empty typed runtime builder.
    #[must_use]
    pub fn builder() -> RuntimeBuilder {
        RuntimeBuilder::default()
    }
    /// Execute a plan and return its useful value.
    pub fn run<P: Plan>(&self, plan: P, context: &crate::Context) -> Result<P::Output, Error> {
        Ok(plan.execute(&self.inner, context)?.value)
    }
    /// Execute a plan with receipt and revision metadata.
    pub fn execute<P: Plan>(
        &self,
        plan: P,
        context: &crate::Context,
    ) -> Result<Execution<P::Output>, Error> {
        plan.execute(&self.inner, context)
    }
    /// Explain a typed query under the supplied authority.
    pub fn explain<T: Object>(
        &self,
        plan: &Query<T>,
        context: &crate::Context,
    ) -> Result<raw::ExecutionPlan, Error> {
        self.inner.explain(context, &T::api_name(), &plan.raw)
    }
    /// Actor-specific capability catalog.
    pub fn capabilities(
        &self,
        context: &crate::Context,
        task: Option<&str>,
    ) -> Result<raw::capability::CapabilityCatalog, Error> {
        self.inner.capabilities(context, task.unwrap_or_default())
    }
    /// Current immutable semantic schema.
    #[must_use]
    pub fn schema(&self) -> Schema {
        Schema(self.inner.spec())
    }
}

/// Typed runtime builder which assembles the raw ontology and bindings.
#[derive(Default)]
pub struct RuntimeBuilder {
    spec: Spec,
    bindings: BindingsBuilder,
    policy: Option<Arc<dyn raw::PolicyEngine>>,
    max_query_limit: usize,
}

impl RuntimeBuilder {
    /// Bind one object definition to a typed backend.
    #[must_use]
    pub fn object<T: Object>(mut self, backend: impl ObjectBinding<T>) -> Self {
        let definition = T::definition();
        self.bindings
            .bind_object(definition.api_name.clone(), backend.into_raw());
        self.spec.upsert_object_type(definition);
        self
    }
    /// Bind one typed relationship resolver.
    #[must_use]
    pub fn link<L, F>(mut self, resolver: F) -> Self
    where
        L: Link,
        F: Fn(
                &crate::Context,
                <L::Source as Object>::Key,
            ) -> Result<Vec<<L::Target as Object>::Key>, Error>
            + Send
            + Sync
            + 'static,
    {
        let definition = L::definition();
        self.spec.upsert_link_type(definition.clone());
        self.bindings.bind_link(
            definition.api_name,
            Arc::new(TypedLink::<L, F> {
                resolver,
                marker: PhantomData,
            }),
        );
        self
    }
    /// Bind one typed action handler.
    #[must_use]
    pub fn action<A, F>(mut self, handler: F) -> Self
    where
        A: Action,
        F: Fn(&crate::Context, A) -> Result<A::Output, Error> + Send + Sync + 'static,
    {
        let definition = A::definition();
        self.spec.upsert_action(definition.clone());
        self.bindings.bind_action(
            definition.api_name,
            Arc::new(TypedAction::<A, F> {
                handler,
                marker: PhantomData,
            }),
        );
        self
    }
    /// Set the pure policy function.
    #[must_use]
    pub fn policy<F>(mut self, policy: F) -> Self
    where
        F: for<'a> Fn(&raw::PolicyRequest<'a>) -> Result<raw::PolicyDecision, Error>
            + Send
            + Sync
            + 'static,
    {
        self.policy = Some(crate::policy::boxed(policy));
        self
    }
    /// Set the hard maximum page size.
    #[must_use]
    pub fn max_query_limit(mut self, limit: usize) -> Self {
        self.max_query_limit = limit;
        self
    }
    /// Compile, validate, and freeze the runtime.
    pub fn build(self) -> Result<Runtime, Error> {
        let policy = self
            .policy
            .ok_or_else(|| Error::validation("policy is required"))?;
        let mut builder = raw::Runtime::builder(self.spec, self.bindings.build(), policy);
        if self.max_query_limit != 0 {
            builder = builder.max_query_limit(self.max_query_limit);
        }
        Ok(Runtime {
            inner: builder.build()?,
        })
    }
}

struct TypedLink<L: Link, F> {
    resolver: F,
    marker: PhantomData<fn() -> L>,
}
impl<L, F> LinkResolver for TypedLink<L, F>
where
    L: Link,
    F: Fn(
            &crate::Context,
            <L::Source as Object>::Key,
        ) -> Result<Vec<<L::Target as Object>::Key>, Error>
        + Send
        + Sync,
{
    fn linked_keys(
        &self,
        context: &crate::Context,
        _link: &raw::ApiName,
        _target: &raw::ApiName,
        source: &raw::Key,
    ) -> Result<Vec<raw::Key>, Error> {
        let source = serde_json::from_value(source.to_value().into())
            .map_err(|error| Error::bad_request(error.to_string()))?;
        (self.resolver)(context, source)?
            .into_iter()
            .map(|key| encode_key(&key))
            .collect()
    }
}

struct TypedAction<A: Action, F> {
    handler: F,
    marker: PhantomData<fn() -> A>,
}
impl<A, F> ActionExecutor for TypedAction<A, F>
where
    A: Action,
    F: Fn(&crate::Context, A) -> Result<A::Output, Error> + Send + Sync,
{
    fn execute(
        &self,
        context: &crate::Context,
        request: &ActionRequest,
    ) -> Result<ActionResult, Error> {
        let command = A::decode_input(request.input.clone())?;
        let output = (self.handler)(context, command)?;
        Ok(ActionResult {
            status: "success".to_owned(),
            output: Some(A::encode_output(&output)?),
            error: None,
            run_id: request.run_id.clone(),
        })
    }
}
