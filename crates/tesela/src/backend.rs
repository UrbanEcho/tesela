//! Typed host backend adapters.

use std::collections::BTreeMap;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::error::Error;
use crate::model::{Object, encode_key};
use crate::raw::{
    self, ApiName, Backend as RawBackend, BackendCapabilities, ExecutionContext, Key,
    MutationResult, Page as RawPage, Query as RawQuery, Record, Revision, Value,
};

/// Typed update values passed to a repository callback.
#[derive(Debug, Clone)]
pub struct Changes<T: Object> {
    values: BTreeMap<ApiName, Value>,
    marker: PhantomData<fn() -> T>,
}

impl<T: Object> Changes<T> {
    /// Raw values for integrations that need dynamic access.
    #[must_use]
    pub fn values(&self) -> &BTreeMap<ApiName, Value> {
        &self.values
    }
}

type Scan<T> =
    dyn Fn(&ExecutionContext) -> Result<Box<dyn Iterator<Item = T> + Send>, Error> + Send + Sync;
type Get<T> =
    dyn Fn(&ExecutionContext, &<T as Object>::Key) -> Result<Option<T>, Error> + Send + Sync;
type Create<T> = dyn Fn(&ExecutionContext, T) -> Result<T, Error> + Send + Sync;
type Update<T> =
    dyn Fn(&ExecutionContext, &<T as Object>::Key, Changes<T>) -> Result<T, Error> + Send + Sync;
type Delete<T> = dyn Fn(&ExecutionContext, &<T as Object>::Key) -> Result<(), Error> + Send + Sync;

/// Closure-backed repository builder.
pub struct RepositoryBuilder<T: Object> {
    scan: Option<Arc<Scan<T>>>,
    get: Option<Arc<Get<T>>>,
    create: Option<Arc<Create<T>>>,
    update: Option<Arc<Update<T>>>,
    delete: Option<Arc<Delete<T>>>,
}

/// Start a typed repository backend.
#[must_use]
pub fn backend<T: Object>() -> RepositoryBuilder<T> {
    RepositoryBuilder {
        scan: None,
        get: None,
        create: None,
        update: None,
        delete: None,
    }
}

impl<T: Object> RepositoryBuilder<T> {
    /// Register a scan callback.
    #[must_use]
    pub fn scan<I>(
        mut self,
        callback: impl Fn(&ExecutionContext) -> Result<I, Error> + Send + Sync + 'static,
    ) -> Self
    where
        I: IntoIterator<Item = T> + 'static,
        I::IntoIter: Send,
    {
        self.scan = Some(Arc::new(move |context| {
            Ok(Box::new(callback(context)?.into_iter()))
        }));
        self
    }
    /// Register a get callback.
    #[must_use]
    pub fn get(
        mut self,
        callback: impl Fn(&ExecutionContext, &T::Key) -> Result<Option<T>, Error>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.get = Some(Arc::new(callback));
        self
    }
    /// Register a create callback.
    #[must_use]
    pub fn create(
        mut self,
        callback: impl Fn(&ExecutionContext, T) -> Result<T, Error> + Send + Sync + 'static,
    ) -> Self {
        self.create = Some(Arc::new(callback));
        self
    }
    /// Register an update callback.
    #[must_use]
    pub fn update(
        mut self,
        callback: impl Fn(&ExecutionContext, &T::Key, Changes<T>) -> Result<T, Error>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.update = Some(Arc::new(callback));
        self
    }
    /// Register a delete callback.
    #[must_use]
    pub fn delete(
        mut self,
        callback: impl Fn(&ExecutionContext, &T::Key) -> Result<(), Error> + Send + Sync + 'static,
    ) -> Self {
        self.delete = Some(Arc::new(callback));
        self
    }
    /// Freeze this backend.
    #[must_use]
    pub fn build(self) -> Repository<T> {
        Repository(Arc::new(RepositoryAdapter { builder: self }))
    }
}

/// A frozen typed repository backend.
pub struct Repository<T: Object>(Arc<RepositoryAdapter<T>>);

impl<T: Object> Clone for Repository<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

#[doc(hidden)]
pub trait ObjectBinding<T: Object> {
    fn into_raw(self) -> Arc<dyn RawBackend>;
}
impl<T: Object> ObjectBinding<T> for Repository<T> {
    fn into_raw(self) -> Arc<dyn RawBackend> {
        self.0
    }
}

/// Development in-memory backend.
#[derive(Clone)]
pub struct MemoryBackend(Arc<raw::MemoryBackend>);
impl MemoryBackend {
    /// Create an empty backend.
    #[must_use]
    pub fn new() -> Self {
        Self(raw::MemoryBackend::new())
    }
}
impl Default for MemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}
impl<T: Object> ObjectBinding<T> for MemoryBackend {
    fn into_raw(self) -> Arc<dyn RawBackend> {
        self.0
    }
}

struct RepositoryAdapter<T: Object> {
    builder: RepositoryBuilder<T>,
}

impl<T: Object> RawBackend for RepositoryAdapter<T> {
    fn backend_type(&self) -> &str {
        "repository"
    }
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            search: self.builder.scan.is_some(),
            get: self.builder.get.is_some(),
            create: self.builder.create.is_some(),
            update: self.builder.update.is_some(),
            delete: self.builder.delete.is_some(),
            aggregate: false,
            upsert: false,
            batch: false,
            conditional_mutation: self.builder.update.is_some() || self.builder.delete.is_some(),
        }
    }
    fn search(
        &self,
        context: &ExecutionContext,
        _object: &ApiName,
        query: &RawQuery,
    ) -> Result<RawPage, Error> {
        let values = self
            .builder
            .scan
            .as_ref()
            .ok_or_else(|| Error::unsupported("scan"))?(context)?;
        raw::execute_residual_try_owned(values.map(record_from_object), query)
    }
    fn get(
        &self,
        context: &ExecutionContext,
        _object: &ApiName,
        key: &Key,
    ) -> Result<Option<Record>, Error> {
        let key = decode_key::<T>(key)?;
        self.builder
            .get
            .as_ref()
            .ok_or_else(|| Error::unsupported("get"))?(context, &key)?
        .map(record_from_object)
        .transpose()
    }
    fn create(
        &self,
        context: &ExecutionContext,
        _object: &ApiName,
        key: Key,
        values: BTreeMap<ApiName, Value>,
    ) -> Result<MutationResult, Error> {
        let value = T::decode(Record::new(key, values))?;
        let stored = self
            .builder
            .create
            .as_ref()
            .ok_or_else(|| Error::unsupported("create"))?(context, value)?;
        Ok(MutationResult {
            record: Some(record_from_object(stored)?),
            rows_affected: Some(1),
        })
    }
    fn update(
        &self,
        context: &ExecutionContext,
        _object: &ApiName,
        key: &Key,
        values: BTreeMap<ApiName, Value>,
        _expected: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let key = decode_key::<T>(key)?;
        let stored = self
            .builder
            .update
            .as_ref()
            .ok_or_else(|| Error::unsupported("update"))?(
            context,
            &key,
            Changes {
                values,
                marker: PhantomData,
            },
        )?;
        Ok(MutationResult {
            record: Some(record_from_object(stored)?),
            rows_affected: Some(1),
        })
    }
    fn delete(
        &self,
        context: &ExecutionContext,
        _object: &ApiName,
        key: &Key,
        _expected: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let key = decode_key::<T>(key)?;
        self.builder
            .delete
            .as_ref()
            .ok_or_else(|| Error::unsupported("delete"))?(context, &key)?;
        Ok(MutationResult {
            record: None,
            rows_affected: Some(1),
        })
    }
}

fn record_from_object<T: Object>(value: T) -> Result<Record, Error> {
    Ok(Record::new(encode_key(&value.key()?)?, value.encode()?))
}
fn decode_key<T: Object>(key: &Key) -> Result<T::Key, Error> {
    serde_json::from_value(key.to_value().into())
        .map_err(|error| Error::bad_request(error.to_string()))
}

/// Native typed backend extension point.
pub trait Native<T: Object>: Send + Sync {
    /// Stable backend kind used in receipts.
    fn backend_type(&self) -> &str {
        "native"
    }
    /// Search using a validated backend-level query.
    fn search(&self, context: &ExecutionContext, query: &RawQuery) -> Result<Vec<T>, Error>;
    /// Whether aggregate queries can be pushed down.
    fn supports_aggregate(&self) -> bool {
        false
    }
    /// Execute a validated aggregate query natively.
    fn aggregate(
        &self,
        _context: &ExecutionContext,
        _query: &raw::AggregateQuery,
    ) -> Result<raw::AggregateResult, Error> {
        Err(Error::unsupported("aggregate"))
    }
}

struct NativeAdapter<T: Object, B: Native<T>> {
    backend: B,
    marker: PhantomData<fn() -> T>,
}

/// Typed binding wrapper for a pushdown-capable native backend.
pub struct NativeBackend<T: Object, B: Native<T>>(NativeAdapter<T, B>);

/// Bind a typed pushdown backend through [`Runtime::builder`](crate::Runtime::builder).
pub fn native<T: Object, B: Native<T>>(backend: B) -> NativeBackend<T, B> {
    NativeBackend(NativeAdapter {
        backend,
        marker: PhantomData,
    })
}

impl<T: Object, B: Native<T> + 'static> ObjectBinding<T> for NativeBackend<T, B> {
    fn into_raw(self) -> Arc<dyn RawBackend> {
        Arc::new(self.0)
    }
}

impl<T: Object, B: Native<T>> RawBackend for NativeAdapter<T, B> {
    fn backend_type(&self) -> &str {
        self.backend.backend_type()
    }
    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            search: true,
            aggregate: self.backend.supports_aggregate(),
            ..BackendCapabilities::default()
        }
    }
    fn search(
        &self,
        context: &ExecutionContext,
        _object: &ApiName,
        query: &RawQuery,
    ) -> Result<RawPage, Error> {
        Ok(RawPage {
            records: self
                .backend
                .search(context, query)?
                .into_iter()
                .map(record_from_object)
                .collect::<Result<_, _>>()?,
            next_cursor: None,
        })
    }
    fn aggregate(
        &self,
        context: &ExecutionContext,
        _object: &ApiName,
        query: &raw::AggregateQuery,
    ) -> Result<raw::AggregateResult, Error> {
        self.backend.aggregate(context, query)
    }
}
