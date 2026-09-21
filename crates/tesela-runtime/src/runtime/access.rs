use super::*;

impl Runtime {
    /// Compile the least capability surface available to this context.
    pub fn capabilities(
        &self,
        context: &ExecutionContext,
        task: &str,
    ) -> Result<CapabilityCatalog, Error> {
        let snapshot = self.snapshot.load_full();
        let task = task.to_ascii_lowercase();
        let mut capabilities = Vec::new();
        for object in snapshot.ontology.object_types() {
            let backend = snapshot
                .objects
                .get(object.id().index())
                .and_then(Option::as_ref);
            for (suffix, operation, supported) in [
                (
                    "search",
                    Operation::Search,
                    backend.is_some_and(|item| item.capabilities().search),
                ),
                (
                    "get",
                    Operation::Read,
                    backend.is_some_and(|item| item.capabilities().get),
                ),
            ] {
                let name = format!("{}.{}", object.api_name(), suffix);
                let policy = supported
                    .then(|| {
                        self.authorize(
                            context,
                            operation,
                            ResourceKind::ObjectType,
                            object.api_name(),
                            None,
                        )
                    })
                    .and_then(Result::ok);
                let allowed = policy.is_some();
                capabilities.push(Capability {
                    exposure: if !allowed {
                        ToolExposure::Hidden
                    } else if task.contains(object.api_name().as_str()) || task.contains(suffix) {
                        ToolExposure::Direct
                    } else {
                        ToolExposure::Discoverable
                    },
                    name,
                    description: format!("{suffix} {} objects", object.api_name()),
                    operation,
                    input_schema: Value::new(serde_json::json!({"type":"object"})),
                    effects: tesela_ir::ActionEffects::read_only(),
                    approval_required: policy
                        .as_ref()
                        .is_some_and(|decision| decision.require_approval),
                    risk: tesela_core::RiskLevel::Low,
                    idempotent: true,
                    concurrency: None,
                    requires_revision: false,
                    preconditions: Vec::new(),
                });
            }
            if let Some(backend) = backend {
                let capabilities_supported = backend.capabilities();
                for (suffix, supported) in [
                    ("create", capabilities_supported.create),
                    ("update", capabilities_supported.update),
                    ("delete", capabilities_supported.delete),
                    ("upsert", capabilities_supported.upsert),
                ] {
                    let name = format!("{}.{}", object.api_name(), suffix);
                    let policy = supported
                        .then(|| {
                            self.authorize(
                                context,
                                Operation::Mutate,
                                ResourceKind::ObjectType,
                                object.api_name(),
                                None,
                            )
                        })
                        .and_then(Result::ok);
                    let allowed = policy.is_some();
                    capabilities.push(Capability {
                        exposure: exposure(allowed, &task, object.api_name(), suffix, true),
                        name,
                        description: format!("{suffix} {} objects", object.api_name()),
                        operation: Operation::Mutate,
                        input_schema: Value::new(serde_json::json!({"type":"object"})),
                        effects: tesela_ir::ActionEffects::new(),
                        approval_required: true,
                        risk: tesela_core::RiskLevel::Medium,
                        idempotent: suffix == "delete",
                        concurrency: None,
                        requires_revision: matches!(suffix, "update" | "delete"),
                        preconditions: Vec::new(),
                    });
                }
            }
        }
        for link in snapshot.ontology.link_types() {
            let bound = snapshot
                .links
                .get(link.id().index())
                .is_some_and(Option::is_some);
            let policy = bound
                .then(|| {
                    self.authorize(
                        context,
                        Operation::Traverse,
                        ResourceKind::LinkType,
                        link.api_name(),
                        None,
                    )
                })
                .and_then(Result::ok);
            let allowed = policy.is_some();
            capabilities.push(Capability {
                name: format!("{}.traverse", link.api_name()),
                description: format!("traverse the {} relationship", link.api_name()),
                exposure: exposure(allowed, &task, link.api_name(), "traverse", false),
                operation: Operation::Traverse,
                input_schema: Value::new(serde_json::json!({"type":"object"})),
                effects: tesela_ir::ActionEffects::read_only(),
                approval_required: policy
                    .as_ref()
                    .is_some_and(|decision| decision.require_approval),
                risk: tesela_core::RiskLevel::Low,
                idempotent: true,
                concurrency: None,
                requires_revision: false,
                preconditions: Vec::new(),
            });
        }
        for action in snapshot.ontology.actions() {
            let bound = snapshot
                .actions
                .get(action.id().index())
                .is_some_and(Option::is_some);
            let policy = bound
                .then(|| {
                    self.authorize(
                        context,
                        Operation::Execute,
                        ResourceKind::Action,
                        action.api_name(),
                        None,
                    )
                })
                .and_then(Result::ok);
            let allowed = policy.is_some();
            capabilities.push(Capability {
                name: action.api_name().to_string(),
                description: action
                    .description()
                    .unwrap_or("semantic action")
                    .to_string(),
                operation: Operation::Execute,
                exposure: if !allowed {
                    ToolExposure::Hidden
                } else if task.contains(action.api_name().as_str()) {
                    ToolExposure::Direct
                } else {
                    ToolExposure::Discoverable
                },
                input_schema: action
                    .input_schema()
                    .cloned()
                    .unwrap_or_else(|| Value::new(serde_json::json!({"type":"object"}))),
                effects: action.effects(),
                approval_required: policy
                    .as_ref()
                    .is_some_and(|decision| decision.require_approval)
                    || !action.is_read_only()
                    || !matches!(action.approval(), tesela_ir::ApprovalPolicy::Never),
                risk: action.risk_level(),
                idempotent: action.effects().idempotent,
                concurrency: Some(action.concurrency()),
                requires_revision: action.requires_revision(),
                preconditions: action.preconditions().to_vec(),
            });
        }
        Ok(CapabilityCatalog {
            generation: snapshot.ontology.generation(),
            fingerprint: snapshot.ontology.fingerprint().clone(),
            capabilities,
        })
    }

    /// Reject an invocation announced under an obsolete ontology snapshot.
    pub fn validate_invocation(&self, invocation: &CapabilityInvocation) -> Result<(), Error> {
        let snapshot = self.snapshot.load();
        if invocation.generation != snapshot.ontology.generation()
            || invocation.fingerprint != *snapshot.ontology.fingerprint()
        {
            return Err(Error::stale_schema("capability catalog is stale"));
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

fn exposure(
    allowed: bool,
    task: &str,
    resource: &ApiName,
    operation: &str,
    program_only: bool,
) -> ToolExposure {
    if !allowed {
        ToolExposure::Hidden
    } else if task.contains(resource.as_str()) || task.contains(operation) {
        ToolExposure::Direct
    } else if program_only {
        ToolExposure::ProgramOnly
    } else {
        ToolExposure::Discoverable
    }
}
