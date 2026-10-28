use super::*;

impl Runtime {
    /// Reject an invocation announced under an obsolete ontology snapshot.
    pub fn validate_invocation(&self, invocation: &CapabilityInvocation) -> Result<(), Error> {
        let snapshot = self.snapshot.load();
        if invocation.generation != snapshot.ontology.generation()
            || invocation.fingerprint != *snapshot.ontology.fingerprint()
        {
            return Err(Error::stale_schema(format!(
                "invocation was built against generation {} but the active ontology is generation {}; \
                 fetch the catalog again with Runtime::capabilities and resubmit with its generation and fingerprint",
                invocation.generation,
                snapshot.ontology.generation(),
            )));
        }
        Ok(())
    }

    /// Explain a query using the exact active snapshot.
    pub fn explain(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        query: &Query,
    ) -> Result<ExecutionPlan, Error> {
        let snapshot = self.snapshot.load_full();
        let object_type = object_type(&snapshot, object)?;
        let decision = self.authorize(
            context,
            Operation::Search,
            ResourceKind::ObjectType,
            object,
            None,
        )?;
        let mut effective = query.clone();
        if let Some(filter) = decision.row_filter {
            effective = effective.and_filter(filter);
        }
        effective.limit = Some(effective_limit(effective.limit, self.max_query_limit) as i32);
        let compiled = CompiledQuery::compile(object_type, &effective, self.max_query_limit)?;
        let backend = object_backend(&snapshot, object_type)?;
        let capabilities = backend.capabilities();
        Ok(ExecutionPlan {
            generation: snapshot.ontology.generation(),
            operation: Operation::Search,
            resource: object.clone(),
            backend: backend.backend_type().to_string(),
            filter: effective.filter,
            sort: effective.sort,
            limit: compiled.limit(),
            filter_pushdown: capabilities.search,
            aggregate_pushdown: capabilities.aggregate,
        })
    }

    /// Explain access without executing the operation.
    pub fn explain_access(
        &self,
        context: &ExecutionContext,
        operation: Operation,
        kind: ResourceKind,
        resource: &ApiName,
        input: Option<&Value>,
    ) -> Result<AccessExplanation, Error> {
        context.check(operation, resource)?;
        let decision = self.policy.evaluate(&PolicyRequest {
            context,
            operation,
            resource_kind: kind,
            resource,
            input,
        })?;
        Ok(AccessExplanation {
            allowed: decision.allow,
            reason: decision.reason,
            row_filter: decision.row_filter,
            redactions: decision.redactions,
            approval_required: decision.require_approval,
            input_constraints: decision.input_constraints,
            transformations: decision.transformations,
            obligations: decision.obligations,
        })
    }

    /// Compare two policy engines against the exact same borrowed request.
    pub fn simulate_policy(
        &self,
        candidate: &dyn PolicyEngine,
        context: &ExecutionContext,
        operation: Operation,
        kind: ResourceKind,
        resource: &ApiName,
        input: Option<&Value>,
    ) -> Result<(AccessExplanation, AccessExplanation), Error> {
        context.check(operation, resource)?;
        let request = PolicyRequest {
            context,
            operation,
            resource_kind: kind,
            resource,
            input,
        };
        let current = self.policy.evaluate(&request)?;
        let candidate = candidate.evaluate(&request)?;
        let convert = |decision: PolicyDecision| AccessExplanation {
            allowed: decision.allow,
            reason: decision.reason,
            row_filter: decision.row_filter,
            redactions: decision.redactions,
            approval_required: decision.require_approval,
            input_constraints: decision.input_constraints,
            transformations: decision.transformations,
            obligations: decision.obligations,
        };
        Ok((convert(current), convert(candidate)))
    }
}
