use std::collections::BTreeMap;
use std::sync::Arc;

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use tesela::backend::{
    ActionExecutor, Backend, BackendCapabilities, BindingsBuilder, LinkResolver,
};
use tesela::dev::MemoryBackend;
use tesela::error::Error;
use tesela::model::{
    ActionResult, AggregateResult, ApiName, Key, MutationResult, Page, Record, Spec, Value,
};
use tesela::policy::{
    AllowAllPolicy, ExecutionContext, PolicyDecision, PolicyEngine, PolicyRequest, Principal,
};
use tesela::query::{ActionRequest, AggregateQuery, Mutation, Query};
use tesela::runtime::Runtime;

use crate::TeselaError;
use crate::conversion::{callback, decode, encode, name, py_to_json, runtime_error};

struct PythonBackend {
    object: Py<PyAny>,
    capabilities: BackendCapabilities,
}
impl Backend for PythonBackend {
    fn backend_type(&self) -> &str {
        "python"
    }
    fn capabilities(&self) -> BackendCapabilities {
        self.capabilities.clone()
    }
    fn search(&self, object: &ApiName, query: &Query) -> Result<Page, Error> {
        callback(
            &self.object,
            "search",
            serde_json::json!({"object_type":object,"query":query}),
        )
    }
    fn get(&self, object: &ApiName, key: &Key) -> Result<Option<Record>, Error> {
        callback(
            &self.object,
            "get",
            serde_json::json!({"object_type":object,"primary_key":key}),
        )
    }
    fn create(
        &self,
        object: &ApiName,
        key: Key,
        values: BTreeMap<ApiName, Value>,
    ) -> Result<MutationResult, Error> {
        callback(
            &self.object,
            "create",
            serde_json::json!({"object_type":object,"primary_key":key,"values":values}),
        )
    }
    fn update(
        &self,
        object: &ApiName,
        key: &Key,
        values: BTreeMap<ApiName, Value>,
        expected: Option<&tesela::model::Revision>,
    ) -> Result<MutationResult, Error> {
        callback(
            &self.object,
            "update",
            serde_json::json!({"object_type":object,"primary_key":key,"values":values,"expected_revision":expected}),
        )
    }
    fn delete(
        &self,
        object: &ApiName,
        key: &Key,
        expected: Option<&tesela::model::Revision>,
    ) -> Result<MutationResult, Error> {
        callback(
            &self.object,
            "delete",
            serde_json::json!({"object_type":object,"primary_key":key,"expected_revision":expected}),
        )
    }
    fn upsert(
        &self,
        object: &ApiName,
        key: Key,
        values: BTreeMap<ApiName, Value>,
        expected: Option<&tesela::model::Revision>,
    ) -> Result<MutationResult, Error> {
        callback(
            &self.object,
            "upsert",
            serde_json::json!({"object_type":object,"primary_key":key,"values":values,"expected_revision":expected}),
        )
    }
    fn batch(
        &self,
        object: &ApiName,
        primary_key: &ApiName,
        mutations: &[Mutation],
    ) -> Result<MutationResult, Error> {
        callback(
            &self.object,
            "batch",
            serde_json::json!({"object_type":object,"primary_key":primary_key,"mutations":mutations}),
        )
    }
    fn aggregate(
        &self,
        object: &ApiName,
        query: &AggregateQuery,
    ) -> Result<AggregateResult, Error> {
        callback(
            &self.object,
            "aggregate",
            serde_json::json!({"object_type":object,"query":query}),
        )
    }
}

struct PythonLink(Py<PyAny>);
impl LinkResolver for PythonLink {
    fn linked_keys(
        &self,
        link: &ApiName,
        target: &ApiName,
        source: &Key,
    ) -> Result<Vec<Key>, Error> {
        callback(
            &self.0,
            "linked_keys",
            serde_json::json!({"link":link,"target":target,"source_key":source}),
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
            serde_json::json!({"principal":context.principal(),"request":request}),
        )
    }
}
struct PythonPolicy(Py<PyAny>);
impl PolicyEngine for PythonPolicy {
    fn evaluate(&self, request: &PolicyRequest<'_>) -> Result<PolicyDecision, Error> {
        callback(
            &self.0,
            "evaluate",
            serde_json::json!({"request":{"principal":request.context.principal(),"operation":request.operation,"resource_kind":request.resource_kind,"resource":request.resource,"input":request.input}}),
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
            let backend: Arc<dyn Backend> = if let Ok(memory) =
                value.extract::<PyRef<'_, NativeMemoryBackend>>()
            {
                memory.inner.clone()
            } else {
                let object = value.unbind();
                let has =
                    |method| Python::attach(|py| object.bind(py).hasattr(method)).unwrap_or(false);
                Arc::new(PythonBackend {
                    capabilities: BackendCapabilities {
                        search: has("search"),
                        get: has("get"),
                        create: has("create"),
                        update: has("update"),
                        delete: has("delete"),
                        upsert: has("upsert"),
                        batch: has("batch"),
                        aggregate: has("aggregate"),
                        conditional_mutation: has("update") || has("delete") || has("upsert"),
                    },
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
        let policy: Arc<dyn PolicyEngine> =
            if policy.extract::<String>().ok().as_deref() == Some("allow_all") {
                Arc::new(AllowAllPolicy)
            } else {
                Arc::new(PythonPolicy(policy.clone().unbind()))
            };
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
}

fn context_from_py(value: &Bound<'_, PyAny>) -> PyResult<ExecutionContext> {
    let data = py_to_json(value)?;
    let id = data
        .get("principal")
        .and_then(|item| item.get("id"))
        .or_else(|| data.get("id"))
        .or_else(|| data.get("user_id"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| PyValueError::new_err("context requires principal.id"))?;
    let mut principal = Principal::new(id);
    if let Some(claims) = data
        .get("principal")
        .and_then(|item| item.get("claims"))
        .or_else(|| data.get("claims"))
    {
        principal.claims = serde_json::from_value(claims.clone())
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
    }
    ExecutionContext::trusted(
        principal,
        data.get("request_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("python"),
    )
    .map_err(runtime_error)
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
