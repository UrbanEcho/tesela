use std::time::{Duration, Instant};

use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use tesela::raw::{ApiName, CancellationToken, ExecutionBudget, ExecutionContext, Principal};

use crate::conversion::{py_to_json, runtime_error};

pub(super) fn context_from_py(value: &Bound<'_, PyAny>) -> PyResult<ExecutionContext> {
    let data = py_to_json(value)?;
    let id = data
        .get("principal")
        .and_then(|item| item.get("id"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| PyValueError::new_err("context requires principal.id"))?;
    let mut principal = Principal::new(id);
    if let Some(claims) = data.get("principal").and_then(|item| item.get("claims")) {
        principal.claims = serde_json::from_value(claims.clone())
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
    }
    let raw_budget = data.get("budget").cloned().unwrap_or_default();
    let budget = ExecutionBudget {
        backend_calls: raw_budget.get("calls").and_then(serde_json::Value::as_u64),
        rows: raw_budget.get("rows").and_then(serde_json::Value::as_u64),
        bytes: raw_budget.get("bytes").and_then(serde_json::Value::as_u64),
        external_cost: raw_budget
            .get("external_cost")
            .and_then(serde_json::Value::as_u64),
        wall_time: raw_budget
            .get("wall_time_seconds")
            .and_then(serde_json::Value::as_f64)
            .map(Duration::from_secs_f64),
    };
    let cancellation = CancellationToken::default();
    if data
        .get("cancelled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        cancellation.cancel();
    }
    let authority = data
        .get("authority")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|error| PyValueError::new_err(error.to_string()))?
        .unwrap_or_default();
    let deadline = data
        .get("deadline_seconds")
        .and_then(serde_json::Value::as_f64)
        .map(|seconds| Instant::now() + Duration::from_secs_f64(seconds.max(0.0)));
    let mut context = ExecutionContext::new(
        principal,
        authority,
        data.get("request_id")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(id),
        deadline,
        cancellation,
        budget,
    )
    .map_err(runtime_error)?;
    if let Some(approvals) = data.get("approvals") {
        let approvals: Vec<String> = serde_json::from_value(approvals.clone())
            .map_err(|error| PyValueError::new_err(error.to_string()))?;
        context =
            context.with_approvals(approvals.into_iter().map(ApiName::new_unchecked).collect());
    }
    Ok(context)
}
