//! Execution-scoped result handles: retention, paging, counting and reference resolution.

use std::sync::Mutex;
use std::time::Instant;

use serde_json::json;

use tesela_core::RiskLevel;

use crate::capability::{Capability, CapabilityAnnotations, ResultHandle, ToolExposure};

use super::*;

/// Requests whose handles are kept before the oldest are evicted.
const MAX_REQUESTS: usize = 256;
/// Handles one request may retain.
const MAX_HANDLES_PER_REQUEST: usize = 32;
/// Rows a single read of a handle returns when the caller gives no limit.
const DEFAULT_READ_LIMIT: usize = 50;
/// Values a `$handle` reference may expand to.
const MAX_REFERENCE_VALUES: usize = 1_000;

/// Capability name that pages the records of a retained handle.
pub(super) const READ_CAPABILITY: &str = "results.read";
/// Capability name that counts the records of a retained handle by a field.
pub(super) const COUNT_BY_CAPABILITY: &str = "results.count_by";

/// Bounded store of retained handles, grouped by request.
#[derive(Debug, Default)]
pub(super) struct HandleStore {
    requests: Mutex<Vec<(String, Vec<ResultHandle>)>>,
}

impl HandleStore {
    fn insert(
        &self,
        context: &ExecutionContext,
        build: impl FnOnce(String) -> Result<ResultHandle, Error>,
    ) -> Result<ResultHandle, Error> {
        let mut requests = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        let request = context.request_id();
        let index = match requests.iter().position(|(id, _)| id == request) {
            Some(index) => index,
            None => {
                if requests.len() >= MAX_REQUESTS {
                    requests.remove(0);
                }
                requests.push((request.to_string(), Vec::new()));
                requests.len() - 1
            }
        };
        let handles = &mut requests[index].1;
        if handles.len() >= MAX_HANDLES_PER_REQUEST {
            return Err(Error::budget_exhausted(format!(
                "a request may retain at most {MAX_HANDLES_PER_REQUEST} result handles"
            )));
        }
        let handle = build(format!("h{}", handles.len() + 1))?;
        handles.push(handle.clone());
        Ok(handle)
    }

    fn get(&self, context: &ExecutionContext, id: &str) -> Result<ResultHandle, Error> {
        let requests = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        requests
            .iter()
            .find(|(request, _)| request == context.request_id())
            .and_then(|(_, handles)| handles.iter().find(|handle| handle.id() == id))
            .cloned()
            .ok_or_else(|| Error::not_found("result handle", id))
    }

    fn release(&self, context: &ExecutionContext) {
        let mut requests = self.requests.lock().unwrap_or_else(|e| e.into_inner());
        requests.retain(|(request, _)| request != context.request_id());
    }
}

/// Input of `results.read`.
#[derive(Deserialize)]
struct ReadInput {
    handle: String,
    #[serde(default)]
    offset: usize,
    #[serde(default)]
    limit: Option<usize>,
    #[serde(default)]
    fields: Option<Vec<ApiName>>,
}

/// Input of `results.count_by`.
#[derive(Deserialize)]
struct CountByInput {
    handle: String,
    field: ApiName,
}

impl Runtime {
    /// Keep records in a handle owned by the context's request.
    ///
    /// Handles carry stable per-request ids (`h1`, `h2`, ...), so the same
    /// sequence of calls yields the same ids on every run.
    pub(super) fn retain_records(
        &self,
        context: &ExecutionContext,
        records: Vec<Record>,
    ) -> Result<ResultHandle, Error> {
        let max_rows = records.len();
        self.handles.insert(context, |id| {
            ResultHandle::new(context, id, records, max_rows)
        })
    }

    /// Fetch a retained handle of this request.
    pub fn handle(&self, context: &ExecutionContext, id: &str) -> Result<ResultHandle, Error> {
        self.handles.get(context, id)
    }

    /// Drop every handle retained for the context's request.
    ///
    /// Hosts call this when a turn ends; handles are otherwise evicted oldest
    /// request first once the store is full.
    pub fn release_handles(&self, context: &ExecutionContext) {
        self.handles.release(context);
    }

    /// Run `results.read` or `results.count_by` against a retained handle.
    pub(super) fn invoke_results(
        &self,
        context: &ExecutionContext,
        capability: &str,
        input: serde_json::Value,
    ) -> Result<Execution<InvocationOutput>, Error> {
        let started = Instant::now();
        let snapshot = self.snapshot.load_full();
        let output = if capability == READ_CAPABILITY {
            let input: ReadInput = serde_json::from_value(input)
                .map_err(|error| Error::validation(error.to_string()))?;
            let handle = self.handles.get(context, &input.handle)?;
            let limit = input.limit.unwrap_or(DEFAULT_READ_LIMIT).clamp(1, 1_000);
            let mut records = handle.page(context, input.offset, limit)?;
            let next = input.offset + records.len();
            if let Some(fields) = &input.fields {
                for record in &mut records {
                    record.values.retain(|name, _| fields.contains(name));
                }
            }
            let next_cursor = (next < handle.len()).then(|| next.to_string());
            InvocationOutput::Page(Page {
                records,
                next_cursor,
            })
        } else {
            let input: CountByInput = serde_json::from_value(input)
                .map_err(|error| Error::validation(error.to_string()))?;
            let handle = self.handles.get(context, &input.handle)?;
            let counts = handle.count_by(context, &input.field)?;
            let groups = counts
                .into_iter()
                .map(|(key, count)| {
                    let mut group = BTreeMap::new();
                    group.insert(input.field.to_string(), Value::new(json!(key)));
                    group.insert("count".to_string(), Value::new(json!(count)));
                    group
                })
                .collect();
            InvocationOutput::Aggregate(AggregateResult { groups })
        };
        let resource = ApiName::new("results")?;
        Ok(self.finish(
            context,
            &snapshot,
            Operation::Read,
            &resource,
            "handles",
            PolicyDecision::default(),
            tesela_ir::ActionEffects::read_only(),
            false,
            output,
            started,
        ))
    }

    /// Replace `{"$handle": id, "field": name}` objects with the array of that
    /// field's values (the primary key when `field` is omitted).
    ///
    /// Lets one capability's result feed the next without the rows passing
    /// through model context.
    pub(super) fn resolve_handle_refs(
        &self,
        context: &ExecutionContext,
        value: &mut serde_json::Value,
    ) -> Result<(), Error> {
        match value {
            serde_json::Value::Object(map) if map.contains_key("$handle") => {
                let id = map
                    .get("$handle")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| Error::validation("$handle must be a handle id string"))?
                    .to_string();
                let field = map
                    .get("field")
                    .map(|field| {
                        field
                            .as_str()
                            .ok_or_else(|| Error::validation("$handle field must be a string"))
                            .and_then(ApiName::new)
                    })
                    .transpose()?;
                let handle = self.handles.get(context, &id)?;
                let records = handle.page(context, 0, MAX_REFERENCE_VALUES + 1)?;
                if records.len() > MAX_REFERENCE_VALUES {
                    return Err(Error::bad_request(format!(
                        "handle {id} has more than {MAX_REFERENCE_VALUES} records and cannot be expanded"
                    )));
                }
                let values = records
                    .into_iter()
                    .filter_map(|record| match &field {
                        Some(field) => record.values.get(field).map(|value| json!(value)),
                        None => Some(json!(record.primary_key)),
                    })
                    .collect();
                *value = serde_json::Value::Array(values);
            }
            serde_json::Value::Object(map) => {
                for child in map.values_mut() {
                    self.resolve_handle_refs(context, child)?;
                }
            }
            serde_json::Value::Array(items) => {
                for child in items {
                    self.resolve_handle_refs(context, child)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

/// The catalog entries that read retained handles. They are request-scoped, so
/// they are listed for every principal and never touch a backend.
pub(super) fn handle_capabilities() -> Vec<Capability> {
    let effects = tesela_ir::ActionEffects::read_only();
    let entry = |name: &str, description: &str, schema: serde_json::Value| Capability {
        name: name.to_string(),
        description: description.to_string(),
        operation: Operation::Read,
        exposure: ToolExposure::Discoverable,
        input_schema: Value::new(schema),
        effects,
        annotations: CapabilityAnnotations {
            read_only: true,
            destructive: false,
            idempotent: true,
            open_world: false,
        },
        approval_required: false,
        risk: RiskLevel::Low,
        concurrency: None,
        requires_revision: false,
        preconditions: Vec::new(),
    };
    vec![
        entry(
            READ_CAPABILITY,
            "page the records kept in a result handle",
            json!({
                "type": "object",
                "required": ["handle"],
                "properties": {
                    "handle": {"type": "string"},
                    "offset": {"type": "integer", "minimum": 0},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 1000},
                    "fields": {"type": "array", "items": {"type": "string"}},
                },
                "additionalProperties": false,
            }),
        ),
        entry(
            COUNT_BY_CAPABILITY,
            "count the records kept in a result handle by a field",
            json!({
                "type": "object",
                "required": ["handle", "field"],
                "properties": {
                    "handle": {"type": "string"},
                    "field": {"type": "string"},
                },
                "additionalProperties": false,
            }),
        ),
    ]
}
