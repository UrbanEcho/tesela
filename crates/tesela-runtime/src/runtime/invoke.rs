//! Invocation of catalog capabilities through the same checked paths as direct calls.

use std::sync::atomic::{AtomicU64, Ordering};

use serde::de::DeserializeOwned;
use serde_json::{Map, json};
use tesela_backend::{ActionRequest, Mutation, Query, TraversalQuery};

use crate::capability::{Capability, CapabilityInvocation, ResultHandle, ToolExposure};

use super::*;

static HANDLE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// The outcome of a capability invocation, shaped for a model to read.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationOutput {
    /// A page of records returned inline.
    Page(Page),
    /// One record returned inline.
    Record(Record),
    /// The outcome of a mutation.
    Mutation(MutationResult),
    /// The outcome of a semantic action.
    Action(ActionResult),
    /// Records kept in an execution-scoped handle; only the summary is returned.
    Handle {
        /// Handle over the retained records.
        handle: ResultHandle,
        /// Continuation cursor for the next page, when one exists.
        next_cursor: Option<String>,
    },
}

/// Key input for a read by primary key.
#[derive(Deserialize)]
struct GetInput {
    key: Key,
}

impl Runtime {
    /// Invoke a capability named in a catalog the caller read.
    ///
    /// The invocation must carry the generation and fingerprint of that catalog.
    /// Hidden and unknown capabilities report as not found. Each route calls the
    /// checked runtime method for its operation, so policy, approval and
    /// revision checks apply exactly as they do to direct calls.
    pub fn invoke(
        &self,
        context: &ExecutionContext,
        invocation: CapabilityInvocation,
    ) -> Result<Execution<InvocationOutput>, Error> {
        self.validate_invocation(&invocation)?;
        let catalog = self.capabilities(context, "")?;
        let capability = catalog
            .capabilities
            .iter()
            .find(|capability| {
                capability.name == invocation.capability
                    && capability.exposure != ToolExposure::Hidden
            })
            .ok_or_else(|| Error::not_found("capability", &invocation.capability))?;
        if capability.requires_revision && invocation.expected_revision.is_none() {
            return Err(Error::bad_request(format!(
                "{} requires expected_revision",
                capability.name
            )));
        }
        let retainable = matches!(
            capability.operation,
            Operation::Search | Operation::Traverse
        );
        if invocation.retain_results && !retainable {
            return Err(Error::bad_request(format!(
                "{} does not support retain_results",
                capability.name
            )));
        }
        let retain = invocation.retain_results;
        match capability.operation {
            Operation::Search => {
                let (object, _) = split_name(capability)?;
                let query: Query = decode(invocation.input.0)?;
                let execution = self.search(context, &object, query)?;
                self.retain_page(context, execution, retain)
            }
            Operation::Read => {
                let (object, _) = split_name(capability)?;
                let input: GetInput = decode(invocation.input.0)?;
                let execution = self.get(context, &object, &input.key)?;
                Ok(wrap(execution, InvocationOutput::Record))
            }
            Operation::Mutate => {
                let (object, suffix) = split_name(capability)?;
                let mutation =
                    mutation_from(suffix, invocation.input.0, invocation.expected_revision)?;
                let execution = self.mutate(context, &object, mutation)?;
                Ok(wrap(execution, InvocationOutput::Mutation))
            }
            Operation::Traverse => {
                let (link, _) = split_name(capability)?;
                let query: TraversalQuery = decode(invocation.input.0)?;
                let execution = self.traverse(context, &link, query)?;
                self.retain_page(context, execution, retain)
            }
            Operation::Execute => {
                let request = ActionRequest {
                    action: ApiName::new(&capability.name)?,
                    input: invocation.input,
                    run_id: invocation.run_id,
                    expected_revision: invocation.expected_revision,
                    preconditions: Vec::new(),
                };
                let execution = self.execute_action(context, request)?;
                Ok(wrap(execution, InvocationOutput::Action))
            }
            _ => Err(Error::unsupported(capability.name.clone())),
        }
    }

    /// Return a page inline, or keep its records in a handle and return the summary.
    fn retain_page(
        &self,
        context: &ExecutionContext,
        execution: Execution<Page>,
        retain: bool,
    ) -> Result<Execution<InvocationOutput>, Error> {
        if !retain {
            return Ok(wrap(execution, InvocationOutput::Page));
        }
        let Page {
            records,
            next_cursor,
        } = execution.output;
        // The handle is bounded by the rows this page already materialized.
        let max_rows = records.len();
        let id = format!("result-{}", HANDLE_SEQUENCE.fetch_add(1, Ordering::Relaxed));
        let handle = ResultHandle::new(context, id, records, max_rows)?;
        Ok(Execution {
            output: InvocationOutput::Handle {
                handle,
                next_cursor,
            },
            receipt: execution.receipt,
        })
    }
}

/// Split a `{resource}.{suffix}` capability name into its resource and suffix.
fn split_name(capability: &Capability) -> Result<(ApiName, &str), Error> {
    let (resource, suffix) = capability
        .name
        .rsplit_once('.')
        .ok_or_else(|| Error::internal("capability name has no operation suffix"))?;
    Ok((ApiName::new(resource)?, suffix))
}

/// Build the externally tagged mutation the backend expects. The revision is
/// supplied by the invocation, never by the body.
fn mutation_from(
    suffix: &str,
    body: serde_json::Value,
    revision: Option<tesela_ir::Revision>,
) -> Result<Mutation, Error> {
    let mut body = body;
    if let Some(revision) = revision {
        if suffix == "create" {
            return Err(Error::bad_request("create does not take expected_revision"));
        }
        let object = body
            .as_object_mut()
            .ok_or_else(|| Error::validation("mutation input must be an object"))?;
        object.insert(
            "expected_revision".to_string(),
            serde_json::to_value(revision).map_err(|error| Error::validation(error.to_string()))?,
        );
    }
    let mut envelope = Map::new();
    envelope.insert(suffix.to_string(), body);
    decode(json!(envelope))
}

fn decode<T: DeserializeOwned>(value: serde_json::Value) -> Result<T, Error> {
    serde_json::from_value(value).map_err(|error| Error::validation(error.to_string()))
}

fn wrap<T>(
    execution: Execution<T>,
    output: impl FnOnce(T) -> InvocationOutput,
) -> Execution<InvocationOutput> {
    Execution {
        output: output(execution.output),
        receipt: execution.receipt,
    }
}
