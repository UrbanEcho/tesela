//! Straight-line programs: validated whole before the first step has an effect.

use std::collections::BTreeSet;

use serde_json::{Value as Json, json};

use crate::capability::{MAX_PROGRAM_STEPS, Program};

use super::*;

/// What one program step did.
#[derive(Debug, Clone, Serialize)]
pub struct StepReport {
    /// Step identifier from the program.
    pub id: String,
    /// Capability the step invoked.
    pub capability: String,
    /// Provenance of the step.
    pub receipt: ExecutionReceipt,
}

/// The result of a program: the last step's output and one report per step.
#[derive(Debug, Clone, Serialize)]
pub struct ProgramOutput {
    /// Output of the last step.
    pub result: InvocationOutput,
    /// Reports for every step that ran, in order.
    pub steps: Vec<StepReport>,
}

impl Runtime {
    /// Validate a program as a whole, then run its steps in order.
    ///
    /// Before any step runs this checks the catalog identity, that every step
    /// names a visible capability, that references point at earlier steps, that
    /// revisions and approvals every step needs are present, and that the
    /// remaining backend-call budget covers the step count. A failure after the
    /// first step names the step and how many completed, so the caller knows
    /// which effects already happened.
    pub fn run_program(
        &self,
        context: &ExecutionContext,
        program: Program,
    ) -> Result<Execution<ProgramOutput>, Error> {
        self.check_program(context, &program)?;
        let count = program.steps.len();
        let mut results: Vec<(String, InvocationOutput)> = Vec::with_capacity(count);
        let mut reports: Vec<StepReport> = Vec::with_capacity(count);
        for (index, step) in program.steps.into_iter().enumerate() {
            let mut input = step.input.0;
            resolve_step_refs(&mut input, &results)?;
            let last = index + 1 == count;
            let invocation = CapabilityInvocation {
                generation: program.generation,
                fingerprint: program.fingerprint.clone(),
                capability: step.capability.clone(),
                input: Value::new(input),
                expected_revision: step.expected_revision,
                run_id: None,
                retain_results: last && program.retain_results,
            };
            let execution = self.invoke(context, invocation).map_err(|error| {
                Error::new(
                    error.code(),
                    format!(
                        "step {} ({}) failed after {index} of {count} step(s) completed: {}",
                        step.id,
                        step.capability,
                        error.message()
                    ),
                )
            })?;
            reports.push(StepReport {
                id: step.id.clone(),
                capability: step.capability,
                receipt: execution.receipt,
            });
            results.push((step.id, execution.output));
        }
        let (_, result) = results
            .pop()
            .ok_or_else(|| Error::bad_request("a program needs at least one step"))?;
        let receipt = reports
            .last()
            .map(|report| report.receipt.clone())
            .ok_or_else(|| Error::internal("program ran no steps"))?;
        Ok(Execution {
            output: ProgramOutput {
                result,
                steps: reports,
            },
            receipt,
        })
    }

    fn check_program(&self, context: &ExecutionContext, program: &Program) -> Result<(), Error> {
        if program.steps.is_empty() {
            return Err(Error::bad_request("a program needs at least one step"));
        }
        if program.steps.len() > MAX_PROGRAM_STEPS {
            return Err(Error::bad_request(format!(
                "a program may have at most {MAX_PROGRAM_STEPS} steps"
            )));
        }
        let catalog = self.capabilities(context, "")?;
        let snapshot = self.snapshot.load();
        if program.generation != snapshot.ontology.generation()
            || program.fingerprint != *snapshot.ontology.fingerprint()
        {
            return Err(Error::stale_schema(format!(
                "program was built against generation {} but the active ontology is generation {}; \
                 fetch the catalog again and resubmit with its generation and fingerprint",
                program.generation,
                snapshot.ontology.generation(),
            )));
        }
        let mut seen = BTreeSet::new();
        let mut unapproved = BTreeSet::new();
        for step in &program.steps {
            let capability = catalog
                .capabilities
                .iter()
                .find(|capability| {
                    capability.name == step.capability
                        && capability.exposure != ToolExposure::Hidden
                })
                .ok_or_else(|| {
                    Error::not_found(
                        "capability",
                        format!("{} (step {})", step.capability, step.id),
                    )
                })?;
            if capability.requires_revision && step.expected_revision.is_none() {
                return Err(Error::bad_request(format!(
                    "step {} ({}) requires expected_revision",
                    step.id, capability.name
                )));
            }
            for reference in step_refs(&step.input.0) {
                if !seen.contains(reference.as_str()) {
                    return Err(Error::bad_request(format!(
                        "step {} references {reference}, which is not an earlier step",
                        step.id
                    )));
                }
            }
            if capability.approval_required {
                let resource = match capability.operation {
                    Operation::Execute => capability.name.as_str(),
                    _ => capability
                        .name
                        .rsplit_once('.')
                        .map_or(capability.name.as_str(), |(resource, _)| resource),
                };
                if !context.is_approved(&ApiName::new(resource)?) {
                    unapproved.insert(resource.to_string());
                }
            }
            if !seen.insert(step.id.clone()) {
                return Err(Error::bad_request(format!(
                    "step id {} is used more than once",
                    step.id
                )));
            }
        }
        if !unapproved.is_empty() {
            return Err(Error::new(
                ErrorCode::ApprovalRequired,
                format!(
                    "the program needs approval for: {}; no step has run",
                    unapproved.into_iter().collect::<Vec<_>>().join(", ")
                ),
            ));
        }
        let steps = program.steps.len() as u64;
        if let Some(calls) = context
            .remaining_budget()
            .backend_calls
            .filter(|calls| steps > *calls)
        {
            return Err(Error::budget_exhausted(format!(
                "the program needs at least {steps} backend calls but {calls} remain; no step has run"
            )));
        }
        Ok(())
    }
}

/// The step ids a JSON input references through `{"$ref": id}`.
fn step_refs(input: &Json) -> Vec<String> {
    match input {
        Json::Object(map) => match map.get("$ref").and_then(Json::as_str) {
            Some(id) => vec![id.to_string()],
            None => map.values().flat_map(step_refs).collect(),
        },
        Json::Array(items) => items.iter().flat_map(step_refs).collect(),
        _ => Vec::new(),
    }
}

/// Replace `{"$ref": id, "field": name}` with the referenced step's values.
fn resolve_step_refs(
    input: &mut Json,
    results: &[(String, InvocationOutput)],
) -> Result<(), Error> {
    match input {
        Json::Object(map) if map.contains_key("$ref") => {
            let id = map
                .get("$ref")
                .and_then(Json::as_str)
                .ok_or_else(|| Error::validation("$ref must be a step id string"))?;
            let field = map.get("field").and_then(Json::as_str);
            let output = results
                .iter()
                .find(|(step, _)| step == id)
                .map(|(_, output)| output)
                .ok_or_else(|| Error::not_found("step result", id))?;
            *input = Json::Array(referenced_values(output, field, id)?);
        }
        Json::Object(map) => {
            for child in map.values_mut() {
                resolve_step_refs(child, results)?;
            }
        }
        Json::Array(items) => {
            for child in items {
                resolve_step_refs(child, results)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn referenced_values(
    output: &InvocationOutput,
    field: Option<&str>,
    id: &str,
) -> Result<Vec<Json>, Error> {
    let record_value = |record: &Record| match field {
        Some(field) => record.get(field).map(|value| json!(value)),
        None => Some(json!(record.primary_key)),
    };
    match output {
        InvocationOutput::Page(page) => Ok(page.records.iter().filter_map(record_value).collect()),
        InvocationOutput::Record(record) => Ok(record_value(record).into_iter().collect()),
        InvocationOutput::Aggregate(result) => {
            let field = field.ok_or_else(|| {
                Error::validation(format!("referencing aggregate step {id} needs a field"))
            })?;
            Ok(result
                .groups
                .iter()
                .filter_map(|group| group.get(field).map(|value| json!(value)))
                .collect())
        }
        _ => Err(Error::bad_request(format!(
            "step {id} produced a result that cannot be referenced"
        ))),
    }
}
