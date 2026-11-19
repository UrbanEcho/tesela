//! Execution-scoped result handles: retention, paging, counting and reference resolution.

use std::collections::VecDeque;
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::time::Instant;

use serde_json::json;

use tesela_core::RiskLevel;

use crate::capability::{Capability, CapabilityAnnotations, ResultHandle, ToolExposure};

use super::*;

/// Handles one request may retain. This is an explicit resource error, not a
/// data cap: a request that needs more results reads them page by page.
const MAX_HANDLES_PER_REQUEST: usize = 32;
/// Default page size of a handle read that gives no limit.
const DEFAULT_READ_LIMIT: usize = 50;

/// Capability name that pages the records of a retained handle.
pub(super) const READ_CAPABILITY: &str = "results.read";
/// Capability name that counts the records of a retained handle by a field.
pub(super) const COUNT_BY_CAPABILITY: &str = "results.count_by";

/// Bounded store of retained handles, grouped by request.
#[derive(Debug)]
pub(super) struct HandleStore {
    capacity: usize,
    state: Mutex<StoreState>,
}

/// Live requests, oldest first, and the ids of the most recently evicted ones.
#[derive(Debug, Default)]
struct StoreState {
    requests: Vec<(String, Vec<ResultHandle>)>,
    /// Bounded to the store capacity so an evicted handle reports expiry.
    evicted: VecDeque<String>,
}

impl HandleStore {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity,
            state: Mutex::default(),
        }
    }

    fn insert(
        &self,
        context: &ExecutionContext,
        build: impl FnOnce(String) -> Result<ResultHandle, Error>,
    ) -> Result<ResultHandle, Error> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let StoreState { requests, evicted } = &mut *state;
        let request = context.request_id();
        let index = match requests.iter().position(|(id, _)| id == request) {
            Some(index) => index,
            None => {
                if requests.len() >= self.capacity {
                    let (oldest, _) = requests.remove(0);
                    if evicted.len() >= self.capacity {
                        evicted.pop_front();
                    }
                    evicted.push_back(oldest);
                }
                evicted.retain(|id| id != request);
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
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .requests
            .iter()
            .find(|(request, _)| request == context.request_id())
            .and_then(|(_, handles)| handles.iter().find(|handle| handle.id() == id))
            .cloned()
            .ok_or_else(|| {
                if state
                    .evicted
                    .iter()
                    .any(|request| request == context.request_id())
                {
                    Error::new(
                        ErrorCode::NotFound,
                        format!(
                            "result handle {id} expired: its request was evicted; \
                             re-run the search and follow next_cursor"
                        ),
                    )
                } else {
                    Error::not_found("result handle", id)
                }
            })
    }

    fn release(&self, context: &ExecutionContext) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state
            .requests
            .retain(|(request, _)| request != context.request_id());
    }
}

/// Input of `results.read`.
#[derive(Deserialize)]
struct ReadInput {
    handle: String,
    #[serde(default)]
    cursor: Option<Cursor>,
    #[serde(default)]
    limit: Option<i64>,
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
        sort: &[Sort],
        partial: bool,
    ) -> Result<ResultHandle, Error> {
        self.handles.insert(context, |id| {
            ResultHandle::new(context, id, records, sort).map(|handle| handle.with_partial(partial))
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
            if let Some(cursor) = &input.cursor {
                cursor.check_token()?;
            }
            let limit = effective_limit(
                Some(
                    input
                        .limit
                        .unwrap_or_else(|| DEFAULT_READ_LIMIT.min(self.max_query_limit) as i64),
                ),
                self.max_query_limit,
            )?;
            let limit = NonZeroUsize::new(limit)
                .ok_or_else(|| Error::internal("a validated limit is at least 1"))?;
            let (mut records, next_cursor) = handle.page(context, input.cursor.as_ref(), limit)?;
            if let Some(fields) = &input.fields {
                for record in &mut records {
                    record.values.retain(|name, _| fields.contains(name));
                }
            }
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
                // Every record is expanded, never truncated. A handle that is
                // only one page of a larger query cannot honor that, so it is
                // refused rather than expanded into a partial key list.
                if handle.is_partial() {
                    return Err(Error::validation(format!(
                        "handle '{id}' holds one page of a larger result and `$handle` \
                         cannot follow cursors; reference the query with a program \
                         `$ref`, which follows every cursor, or read the handle with \
                         `results.read` and follow its next_cursor"
                    )));
                }
                let values = handle
                    .iter(context)?
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
pub(super) fn handle_capabilities(max_limit: usize) -> Vec<Capability> {
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
                    "limit": {"type": "integer", "minimum": 1, "maximum": max_limit},
                    "cursor": {
                        "type": "string",
                        "description": "Opaque token: the `next_cursor` of a previous read of this handle. Omit for the first page.",
                    },
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
