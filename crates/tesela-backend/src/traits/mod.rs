//! Host backend, action, link, binding, policy, and observation contracts.

use crate::{
    ActionRequest, AggregateQuery, BackendCapabilities, Mutation, PolicyDecision, PolicyRequest,
    Query,
};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use tesela_core::{ApiName, Error, ExecutionContext, Key, Value};
use tesela_ir::{ActionResult, AggregateResult, MutationResult, Page, Record, Revision};

/// A host-owned data backend. Every operation is optional.
pub trait Backend: Send + Sync {
    /// Stable diagnostic name.
    fn backend_type(&self) -> &str;
    /// Supported native operations.
    fn capabilities(&self) -> BackendCapabilities;
    /// Search records.
    fn search(&self, object: &ApiName, query: &Query) -> Result<Page, Error> {
        let _ = (object, query);
        Err(Error::unsupported("search"))
    }
    /// Fetch one record.
    fn get(&self, object: &ApiName, key: &Key) -> Result<Option<Record>, Error> {
        let _ = (object, key);
        Err(Error::unsupported("get"))
    }
    /// Create one record.
    fn create(
        &self,
        object: &ApiName,
        key: Key,
        values: BTreeMap<ApiName, Value>,
    ) -> Result<MutationResult, Error> {
        let _ = (object, key, values);
        Err(Error::unsupported("create"))
    }
    /// Conditionally update one record.
    fn update(
        &self,
        object: &ApiName,
        key: &Key,
        values: BTreeMap<ApiName, Value>,
        expected: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let _ = (object, key, values, expected);
        Err(Error::unsupported("update"))
    }
    /// Conditionally delete one record.
    fn delete(
        &self,
        object: &ApiName,
        key: &Key,
        expected: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let _ = (object, key, expected);
        Err(Error::unsupported("delete"))
    }
    /// Native atomic upsert.
    fn upsert(
        &self,
        object: &ApiName,
        key: Key,
        values: BTreeMap<ApiName, Value>,
        expected: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let _ = (object, key, values, expected);
        Err(Error::unsupported("atomic upsert"))
    }
    /// Native atomic batch.
    fn batch(
        &self,
        object: &ApiName,
        primary_key: &ApiName,
        mutations: &[Mutation],
    ) -> Result<MutationResult, Error> {
        let _ = (object, primary_key, mutations);
        Err(Error::unsupported("atomic batch"))
    }
    /// Native aggregate pushdown.
    fn aggregate(
        &self,
        object: &ApiName,
        query: &AggregateQuery,
    ) -> Result<AggregateResult, Error> {
        let _ = (object, query);
        Err(Error::unsupported("aggregate"))
    }
    /// Release resources.
    fn close(&self) -> Result<(), Error> {
        Ok(())
    }
}

/// Resolve a semantic link to target primary keys.
pub trait LinkResolver: Send + Sync {
    /// Resolve linked keys.
    fn linked_keys(
        &self,
        link: &ApiName,
        target: &ApiName,
        source: &Key,
    ) -> Result<Vec<Key>, Error>;
}

/// Closure-backed link resolver.
pub struct FnLinkResolver<F> {
    resolve: F,
}
impl<F> FnLinkResolver<F>
where
    F: Fn(&ApiName, &ApiName, &Key) -> Result<Vec<Key>, Error> + Send + Sync + 'static,
{
    /// Wrap a closure.
    #[must_use]
    pub fn new(resolve: F) -> Arc<Self> {
        Arc::new(Self { resolve })
    }
}
impl<F> LinkResolver for FnLinkResolver<F>
where
    F: Fn(&ApiName, &ApiName, &Key) -> Result<Vec<Key>, Error> + Send + Sync,
{
    fn linked_keys(
        &self,
        link: &ApiName,
        target: &ApiName,
        source: &Key,
    ) -> Result<Vec<Key>, Error> {
        (self.resolve)(link, target, source)
    }
}

/// Execute one separately bound semantic action.
pub trait ActionExecutor: Send + Sync {
    /// Execute after runtime checks succeed.
    fn execute(
        &self,
        context: &ExecutionContext,
        request: &ActionRequest,
    ) -> Result<ActionResult, Error>;
}

/// Closure-backed action executor.
pub struct FnActionExecutor<F> {
    execute: F,
}
impl<F> FnActionExecutor<F>
where
    F: Fn(&ExecutionContext, &ActionRequest) -> Result<ActionResult, Error> + Send + Sync + 'static,
{
    /// Wrap a closure.
    #[must_use]
    pub fn new(execute: F) -> Arc<Self> {
        Arc::new(Self { execute })
    }
}
impl<F> ActionExecutor for FnActionExecutor<F>
where
    F: Fn(&ExecutionContext, &ActionRequest) -> Result<ActionResult, Error> + Send + Sync,
{
    fn execute(
        &self,
        context: &ExecutionContext,
        request: &ActionRequest,
    ) -> Result<ActionResult, Error> {
        (self.execute)(context, request)
    }
}

type SearchFn = dyn Fn(&ApiName, &Query) -> Result<Page, Error> + Send + Sync;
type GetFn = dyn Fn(&ApiName, &Key) -> Result<Option<Record>, Error> + Send + Sync;
type CreateFn =
    dyn Fn(&ApiName, Key, BTreeMap<ApiName, Value>) -> Result<MutationResult, Error> + Send + Sync;
type UpdateFn = dyn Fn(&ApiName, &Key, BTreeMap<ApiName, Value>, Option<&Revision>) -> Result<MutationResult, Error>
    + Send
    + Sync;
type DeleteFn =
    dyn Fn(&ApiName, &Key, Option<&Revision>) -> Result<MutationResult, Error> + Send + Sync;
type UpsertFn = dyn Fn(&ApiName, Key, BTreeMap<ApiName, Value>, Option<&Revision>) -> Result<MutationResult, Error>
    + Send
    + Sync;
type BatchFn =
    dyn Fn(&ApiName, &ApiName, &[Mutation]) -> Result<MutationResult, Error> + Send + Sync;
type AggregateFn =
    dyn Fn(&ApiName, &AggregateQuery) -> Result<AggregateResult, Error> + Send + Sync;

mod builder;
pub use builder::*;
