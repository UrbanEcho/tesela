use super::*;

impl Runtime {
    pub(super) fn authorize(
        &self,
        context: &ExecutionContext,
        operation: Operation,
        kind: ResourceKind,
        resource: &ApiName,
        input: Option<&Value>,
    ) -> Result<PolicyDecision, Error> {
        context.check(operation, resource)?;
        let decision = self.policy.evaluate(&PolicyRequest {
            context,
            operation,
            resource_kind: kind,
            resource,
            input,
        })?;
        if decision.allow {
            Ok(decision)
        } else {
            Err(Error::policy_denied(
                decision
                    .reason
                    .unwrap_or_else(|| "policy denied".to_string()),
            ))
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn finish<T>(
        &self,
        context: &ExecutionContext,
        snapshot: &ExecutionSnapshot,
        operation: Operation,
        resource: &ApiName,
        source: &str,
        decision: PolicyDecision,
        effects: tesela_ir::ActionEffects,
        approval_required: bool,
        output: T,
        started: Instant,
    ) -> Execution<T> {
        let receipt = ExecutionReceipt {
            request_id: context.request_id().to_string(),
            generation: snapshot.ontology.generation(),
            ontology: snapshot.ontology.fingerprint().clone(),
            principal: context.principal().id.to_string(),
            operation,
            resource: resource.clone(),
            source: source.to_string(),
            redactions: decision.redactions,
            effects,
            approval_required,
            approval_granted: !approval_required || context.is_approved(resource),
            obligations: decision.obligations,
            elapsed_micros: started.elapsed().as_micros().min(u128::from(u64::MAX)) as u64,
        };
        let event = ExecutionEvent {
            request_id: Arc::from(context.request_id()),
            operation: Arc::from(format!("{operation:?}").to_lowercase()),
            resource: resource.clone(),
            success: true,
            side_effect: if matches!(operation, Operation::Mutate | Operation::Execute) {
                SideEffectStatus::Completed
            } else {
                SideEffectStatus::None
            },
        };
        for observer in self.observers.iter() {
            let _ = catch_unwind(AssertUnwindSafe(|| observer.observe(&event)));
        }
        Execution { output, receipt }
    }
}
