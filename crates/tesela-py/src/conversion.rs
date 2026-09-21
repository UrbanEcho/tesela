//! Direct Python/value conversion and callback error mapping.

use pyo3::exceptions::{PyRuntimeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PyDict, PyFloat, PyInt, PyList, PyString, PyTuple};
use tesela::Error;
use tesela::raw::ApiName;

use crate::TeselaError;

pub(crate) fn py_to_json(value: &Bound<'_, PyAny>) -> PyResult<serde_json::Value> {
    if value.is_none() {
        return Ok(serde_json::Value::Null);
    }
    if let Ok(value) = value.cast::<PyBool>() {
        return Ok(serde_json::Value::Bool(value.is_true()));
    }
    if let Ok(value) = value.cast::<PyInt>() {
        if let Ok(value) = value.extract::<i64>() {
            return Ok(value.into());
        }
        return value.extract::<u64>().map(serde_json::Value::from);
    }
    if let Ok(value) = value.cast::<PyFloat>() {
        return serde_json::Number::from_f64(value.value())
            .map(serde_json::Value::Number)
            .ok_or_else(|| PyValueError::new_err("non-finite float"));
    }
    if let Ok(value) = value.cast::<PyString>() {
        return Ok(serde_json::Value::String(value.to_str()?.to_string()));
    }
    if let Ok(value) = value.cast::<PyList>() {
        return value
            .iter()
            .map(|item| py_to_json(&item))
            .collect::<PyResult<Vec<_>>>()
            .map(serde_json::Value::Array);
    }
    if let Ok(value) = value.cast::<PyTuple>() {
        return value
            .iter()
            .map(|item| py_to_json(&item))
            .collect::<PyResult<Vec<_>>>()
            .map(serde_json::Value::Array);
    }
    if let Ok(value) = value.cast::<PyDict>() {
        let mut output = serde_json::Map::with_capacity(value.len());
        for (key, item) in value {
            output.insert(key.extract::<String>()?, py_to_json(&item)?);
        }
        return Ok(serde_json::Value::Object(output));
    }
    let value_type = value.get_type();
    let module = value_type
        .getattr("__module__")?
        .extract::<String>()
        .unwrap_or_default();
    let type_name = value_type.name()?.to_string();
    if module == "enum" || value.hasattr("value")? && value.hasattr("name")? {
        return py_to_json(&value.getattr("value")?);
    }
    if module == "uuid" || module == "decimal" {
        return Ok(serde_json::Value::String(
            value.str()?.to_str()?.to_string(),
        ));
    }
    if module == "datetime" && matches!(type_name.as_str(), "date" | "datetime") {
        return Ok(serde_json::Value::String(
            value.call_method0("isoformat")?.extract()?,
        ));
    }
    if value.hasattr("__dict__")? {
        return py_to_json(&value.getattr("__dict__")?);
    }
    Err(PyValueError::new_err(format!(
        "unsupported Python value {}",
        value.get_type().name()?
    )))
}

fn json_to_py(py: Python<'_>, value: &serde_json::Value) -> PyResult<Py<PyAny>> {
    Ok(match value {
        serde_json::Value::Null => py.None(),
        serde_json::Value::Bool(value) => value.into_pyobject(py)?.to_owned().unbind().into_any(),
        serde_json::Value::Number(value) if value.is_i64() => value
            .as_i64()
            .unwrap_or_default()
            .into_pyobject(py)?
            .unbind()
            .into_any(),
        serde_json::Value::Number(value) if value.is_u64() => value
            .as_u64()
            .unwrap_or_default()
            .into_pyobject(py)?
            .unbind()
            .into_any(),
        serde_json::Value::Number(value) => value
            .as_f64()
            .unwrap_or_default()
            .into_pyobject(py)?
            .unbind()
            .into_any(),
        serde_json::Value::String(value) => value.into_pyobject(py)?.unbind().into_any(),
        serde_json::Value::Array(values) => PyList::new(
            py,
            values
                .iter()
                .map(|item| json_to_py(py, item))
                .collect::<PyResult<Vec<_>>>()?,
        )?
        .unbind()
        .into_any(),
        serde_json::Value::Object(values) => {
            let output = PyDict::new(py);
            for (key, item) in values {
                output.set_item(key, json_to_py(py, item)?)?;
            }
            output.unbind().into_any()
        }
    })
}

pub(crate) fn decode<T: serde::de::DeserializeOwned>(value: &Bound<'_, PyAny>) -> PyResult<T> {
    serde_json::from_value(py_to_json(value)?)
        .map_err(|error| PyValueError::new_err(error.to_string()))
}

pub(crate) fn encode<T: serde::Serialize>(py: Python<'_>, value: &T) -> PyResult<Py<PyAny>> {
    json_to_py(
        py,
        &serde_json::to_value(value).map_err(|error| PyRuntimeError::new_err(error.to_string()))?,
    )
}

pub(crate) fn name(value: &str) -> PyResult<ApiName> {
    ApiName::new(value).map_err(|error| PyValueError::new_err(error.to_string()))
}

pub(crate) fn runtime_error(error: Error) -> PyErr {
    let code = error.code().as_str();
    let retryable = error.is_retryable();
    let side_effect = error.side_effect().as_str();
    let recovery = error.recovery().map(tesela::raw::Recovery::as_str);
    let resource_kind = error.resource_kind().map(|kind| kind.as_str());
    let resource = error.resource();
    let exception = PyErr::new::<TeselaError, _>(error.to_string());
    Python::attach(|py| {
        let value = exception.value(py);
        let _ = value.setattr("code", code);
        let _ = value.setattr("retryable", retryable);
        let _ = value.setattr("side_effect", side_effect);
        let _ = value.setattr("recovery", recovery);
        let _ = value.setattr("resource_kind", resource_kind);
        let _ = value.setattr("resource", resource);
    });
    exception
}

pub(crate) fn callback<T: serde::de::DeserializeOwned>(
    object: &Py<PyAny>,
    method: &str,
    payload: serde_json::Value,
) -> Result<T, Error> {
    Python::attach(|py| {
        let input = json_to_py(py, &payload).map_err(|error| Error::adapter(error.to_string()))?;
        let output = object
            .call_method1(py, method, (input,))
            .map_err(|error| Error::adapter(format!("Python {method}: {error}")))?;
        serde_json::from_value(
            py_to_json(output.bind(py)).map_err(|error| Error::adapter(error.to_string()))?,
        )
        .map_err(|error| Error::adapter(format!("Python {method} returned invalid data: {error}")))
    })
}
