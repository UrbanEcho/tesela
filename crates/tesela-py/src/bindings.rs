use pyo3::prelude::*;
use pyo3::types::PyDict;
use std::collections::BTreeMap;
use std::sync::Arc;
use tesela::Error;
use tesela::raw::MemoryBackend;
use tesela::raw::{ActionExecutor, Backend, BackendCapabilities, BindingsBuilder, LinkResolver};
use tesela::raw::{ActionRequest, AggregateQuery, Mutation, Query, Runtime};
use tesela::raw::{
    ActionResult, AggregateResult, ApiName, Key, MutationResult, Page, Record, Spec, Value,
};
use tesela::raw::{ExecutionContext, PolicyDecision, PolicyEngine, PolicyRequest};

use crate::TeselaError;
use crate::conversion::{callback, decode, encode, name, runtime_error};

#[path = "python_context.rs"]
mod python_context;
use python_context::context_from_py;

fn context_payload(context: &ExecutionContext) -> serde_json::Value {
    serde_json::json!({
        "principal": context.principal(),
        "request_id": context.request_id(),
        "approvals": context.approvals(),
        "authority": context.authority(),
        "deadline_seconds": context.remaining().map(|duration| duration.as_secs_f64()),
        "budget": {
            "calls": context.budget().backend_calls,
            "rows": context.budget().rows,
            "bytes": context.budget().bytes,
            "external_cost": context.budget().external_cost,
            "wall_time_seconds": context.budget().wall_time.map(|duration| duration.as_secs_f64()),
        },
        "cancelled": context.is_cancelled(),
    })
}

struct PythonBackend {
    object: Py<PyAny>,
    capabilities: BackendCapabilities,
    scan: bool,
}
impl Backend for PythonBackend {
    fn backend_type(&self) -> &str {
        "python"
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities.clone()
    }
    fn search(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        query: &Query,
    ) -> Result<Page, Error> {
        let payload = serde_json::json!({"context":context_payload(context),"object_type":object,"query":query});
        if self.scan {
            let records: Vec<Record> = callback(&self.object, "scan", payload)?;
            tesela::raw::execute_residual(records.iter(), query)
        } else {
            callback(&self.object, "search", payload)
        }
    }
    fn get(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        key: &Key,
    ) -> Result<Option<Record>, Error> {
        callback(
            &self.object,
            "get",
            serde_json::json!({"context":context_payload(context),"object_type":object,"primary_key":key}),
        )
    }
    fn create(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        key: Key,
        values: BTreeMap<ApiName, Value>,
    ) -> Result<MutationResult, Error> {
        callback(
            &self.object,
            "create",
            serde_json::json!({"context":context_payload(context),"object_type":object,"primary_key":key,"values":values}),
        )
    }
    fn update(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        key: &Key,
        values: BTreeMap<ApiName, Value>,
        expected: Option<&tesela::raw::Revision>,
    ) -> Result<MutationResult, Error> {
        callback(
            &self.object,
            "update",
            serde_json::json!({"context":context_payload(context),"object_type":object,"primary_key":key,"values":values,"expected_revision":expected}),
        )
    }
    fn delete(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        key: &Key,
        expected: Option<&tesela::raw::Revision>,
    ) -> Result<MutationResult, Error> {
        callback(
            &self.object,
            "delete",
            serde_json::json!({"context":context_payload(context),"object_type":object,"primary_key":key,"expected_revision":expected}),
        )
    }
    fn upsert(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        key: Key,
        values: BTreeMap<ApiName, Value>,
        expected: Option<&tesela::raw::Revision>,
    ) -> Result<MutationResult, Error> {
        callback(
            &self.object,
            "upsert",
            serde_json::json!({"context":context_payload(context),"object_type":object,"primary_key":key,"values":values,"expected_revision":expected}),
        )
    }
    fn batch(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        primary_key: &ApiName,
        mutations: &[Mutation],
    ) -> Result<MutationResult, Error> {
        callback(
            &self.object,
            "batch",
            serde_json::json!({"context":context_payload(context),"object_type":object,"primary_key":primary_key,"mutations":mutations}),
        )
    }
    fn aggregate(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        query: &AggregateQuery,
    ) -> Result<AggregateResult, Error> {
        callback(
            &self.object,
            "aggregate",
            serde_json::json!({"context":context_payload(context),"object_type":object,"query":query}),
        )
    }
}

struct PythonLink(Py<PyAny>);
impl LinkResolver for PythonLink {
    fn linked_keys(
        &self,
        context: &ExecutionContext,
        link: &ApiName,
        target: &ApiName,
        source: &Key,
    ) -> Result<Vec<Key>, Error> {
        callback(
            &self.0,
            "linked_keys",
            serde_json::json!({"context":context_payload(context),"link":link,"target":target,"source_key":source}),
        )
    }
}
struct PythonAction(Py<PyAny>);
impl ActionExecutor for PythonAction {
    fn execute(
        &self,
        context: &ExecutionContext,
        request: &ActionRequest,
    ) -> Result<ActionResult, Error> {
        callback(
            &self.0,
            "execute",
            serde_json::json!({"context":context_payload(context),"request":request}),
        )
    }
}
struct PythonPolicy(Py<PyAny>);
impl PolicyEngine for PythonPolicy {
    fn evaluate(&self, request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error> {
        callback(
            &self.0,
            "evaluate",
            serde_json::json!({"request":{"context":context_payload(request.context),"operation":request.operation,"resource_kind":request.resource_kind,"resource":request.resource,"input":request.input}}),
        )
    }
}

#[pyclass]
struct NativeMemoryBackend {
    inner: Arc<MemoryBackend>,
}
#[pymethods]
impl NativeMemoryBackend {
    #[new]
    fn new() -> Self {
        Self {
            inner: MemoryBackend::new(),
        }
    }
    fn bind_link(&self, link: &str, target_property: &str) -> PyResult<()> {
        self.inner.bind_link(name(link)?, name(target_property)?);
        Ok(())
    }
}

#[pyclass]
struct NativeRuntime {
    inner: Arc<Runtime>,
}
#[pymethods]
impl NativeRuntime {
    #[new]
    #[pyo3(signature = (spec, objects, policy, links=None, actions=None, max_query_limit=1000))]
    fn new(
        spec: &Bound<'_, PyAny>,
        objects: &Bound<'_, PyDict>,
        policy: &Bound<'_, PyAny>,
        links: Option<&Bound<'_, PyDict>>,
        actions: Option<&Bound<'_, PyDict>>,
        max_query_limit: usize,
    ) -> PyResult<Self> {
        let spec: Spec = decode(spec)?;
        let mut bindings = BindingsBuilder::new();
        for (key, value) in objects {
            let object = name(&key.extract::<String>()?)?;
            let backend: Arc<dyn Backend> =
                if let Ok(memory) = value.extract::<PyRef<'_, NativeMemoryBackend>>() {
                    memory.inner.clone()
                } else {
                    let object = value.unbind();
                    let declared = Python::attach(|py| {
                        object
                            .bind(py)
                            .getattr("__tesela_capabilities__")
                            .and_then(|value| value.extract::<Vec<String>>())
                    })
                    .unwrap_or_default();
                    let has = |method: &str| {
                        declared.iter().any(|value| value == method)
                            || (declared.is_empty()
                                && Python::attach(|py| object.bind(py).hasattr(method))
                                    .unwrap_or(false))
                    };
                    Arc::new(PythonBackend {
                        capabilities: BackendCapabilities {
                            search: has("search") || has("scan"),
                            get: has("get"),
                            create: has("create"),
                            update: has("update"),
                            delete: has("delete"),
                            upsert: has("upsert"),
                            batch: has("batch"),
                            aggregate: has("aggregate"),
                            conditional_mutation: has("update") || has("delete") || has("upsert"),
                        },
                        scan: has("scan") && !has("search"),
                        object,
                    })
                };
            bindings.bind_object(object, backend);
        }
        if let Some(links) = links {
            for (key, value) in links {
                bindings.bind_link(
                    name(&key.extract::<String>()?)?,
                    Arc::new(PythonLink(value.unbind())),
                );
            }
        }
        if let Some(actions) = actions {
            for (key, value) in actions {
                bindings.bind_action(
                    name(&key.extract::<String>()?)?,
                    Arc::new(PythonAction(value.unbind())),
                );
            }
        }
        let policy: Arc<dyn PolicyEngine> = Arc::new(PythonPolicy(policy.clone().unbind()));
        Ok(Self {
            inner: Runtime::builder(spec, bindings.build(), policy)
                .max_query_limit(max_query_limit)
                .build()
                .map_err(runtime_error)?,
        })
    }
    fn spec(&self, py: Python<'_>) -> PyResult<Py<PyAny>> {
        encode(py, &*self.inner.spec())
    }
    fn apply_spec(&self, spec: &Bound<'_, PyAny>) -> PyResult<()> {
        self.inner.apply_spec(decode(spec)?).map_err(runtime_error)
    }
    fn search(
        &self,
        py: Python<'_>,
        context: &Bound<'_, PyAny>,
        object: &str,
        query: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let context = context_from_py(context)?;
        encode(
            py,
            &self
                .inner
                .search(&context, &name(object)?, decode(query)?)
                .map_err(runtime_error)?,
        )
    }
    fn get(
        &self,
        py: Python<'_>,
        context: &Bound<'_, PyAny>,
        object: &str,
        key: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let context = context_from_py(context)?;
        encode(
            py,
            &self
                .inner
                .get(&context, &name(object)?, &decode(key)?)
                .map_err(runtime_error)?,
        )
    }
    fn lookup(
        &self,
        py: Python<'_>,
        context: &Bound<'_, PyAny>,
        object: &str,
        key: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let context = context_from_py(context)?;
        encode(
            py,
            &self
                .inner
                .lookup(&context, &name(object)?, &decode(key)?)
                .map_err(runtime_error)?,
        )
    }
    fn mutate(
        &self,
        py: Python<'_>,
        context: &Bound<'_, PyAny>,
        object: &str,
        mutation: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let context = context_from_py(context)?;
        encode(
            py,
            &self
                .inner
                .mutate(&context, &name(object)?, decode(mutation)?)
                .map_err(runtime_error)?,
        )
    }
    fn aggregate(
        &self,
        py: Python<'_>,
        context: &Bound<'_, PyAny>,
        object: &str,
        query: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let context = context_from_py(context)?;
        encode(
            py,
            &self
                .inner
                .aggregate(&context, &name(object)?, decode(query)?)
                .map_err(runtime_error)?,
        )
    }
    fn traverse(
        &self,
        py: Python<'_>,
        context: &Bound<'_, PyAny>,
        link: &str,
        query: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let context = context_from_py(context)?;
        encode(
            py,
            &self
                .inner
                .traverse(&context, &name(link)?, decode(query)?)
                .map_err(runtime_error)?,
        )
    }
    fn execute_action(
        &self,
        py: Python<'_>,
        context: &Bound<'_, PyAny>,
        request: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        let context = context_from_py(context)?;
        encode(
            py,
            &self
                .inner
                .execute_action(&context, decode(request)?)
                .map_err(runtime_error)?,
        )
    }
    fn capabilities(
        &self,
        py: Python<'_>,
        context: &Bound<'_, PyAny>,
        task: &str,
    ) -> PyResult<Py<PyAny>> {
        encode(
            py,
            &self
                .inner
                .capabilities(&context_from_py(context)?, task)
                .map_err(runtime_error)?,
        )
    }
    fn explain(
        &self,
        py: Python<'_>,
        context: &Bound<'_, PyAny>,
        object: &str,
        query: &Bound<'_, PyAny>,
    ) -> PyResult<Py<PyAny>> {
        encode(
            py,
            &self
                .inner
                .explain(&context_from_py(context)?, &name(object)?, &decode(query)?)
                .map_err(runtime_error)?,
        )
    }
}

#[pyfunction]
fn normalize_spec(py: Python<'_>, spec: &Bound<'_, PyAny>) -> PyResult<Py<PyAny>> {
    let spec: Spec = decode(spec)?;
    encode(py, &spec)
}

pub(crate) fn register(module: &Bound<'_, PyModule>) -> PyResult<()> {
    module.add("TeselaError", module.py().get_type::<TeselaError>())?;
    module.add_class::<NativeMemoryBackend>()?;
    module.add_class::<NativeRuntime>()?;
    module.add_function(wrap_pyfunction!(normalize_spec, module)?)?;
    Ok(())
}
