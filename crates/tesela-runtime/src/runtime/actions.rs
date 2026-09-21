use super::*;

impl Runtime {
    /// Execute a separately bound action.
    pub fn execute_action(
        &self,
        context: &ExecutionContext,
        mut request: ActionRequest,
    ) -> Result<Execution<ActionResult>, Error> {
        let started = Instant::now();
        let snapshot = self.snapshot.load_full();
        let action = snapshot.ontology.action(&request.action).ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownAction,
                format!("unknown action '{}'", request.action),
            )
        })?;
        let decision = self.authorize(
            context,
            Operation::Execute,
            ResourceKind::Action,
            &request.action,
            Some(&request.input),
        )?;
        if !decision.transformations.is_empty() {
            let input = request
                .input
                .0
                .as_object_mut()
                .ok_or_else(|| Error::bad_request("action input must be an object"))?;
            for (field, value) in &decision.transformations {
                input.insert(field.as_str().to_owned(), value.clone().into());
            }
        }
        if let Some(schema) = action.input_schema() {
            validate_schema(schema, &request.input)?;
        }
        if action.idempotency_key().is_some() && request.run_id.is_none() {
            return Err(Error::new(
                ErrorCode::PreconditionFailed,
                "action requires an idempotency run_id",
            ));
        }
        context.charge_bytes(value_bytes(&request.input))?;
        if action.requires_revision() && request.expected_revision.is_none() {
            return Err(Error::new(
                ErrorCode::PreconditionFailed,
                "action requires an expected revision",
            ));
        }
        if !decision.input_constraints.is_empty() {
            let values = value_fields(&request.input)?;
            let subject = action
                .subject()
                .and_then(|id| snapshot.ontology.object_type_by_id(id))
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::PreconditionFailed,
                        "input constraints require an action subject",
                    )
                })?;
            for constraint in &decision.input_constraints {
                if !compile_filter(subject, constraint)?.matches(&values) {
                    return Err(Error::new(
                        ErrorCode::PreconditionFailed,
                        "action input constraints were not satisfied",
                    ));
                }
            }
        }
        if request.expected_revision.is_some()
            || !request.preconditions.is_empty()
            || !action.preconditions().is_empty()
            || decision.row_filter.is_some()
        {
            let subject = action
                .subject()
                .and_then(|id| snapshot.ontology.object_type_by_id(id))
                .ok_or_else(|| {
                    Error::new(
                        ErrorCode::PreconditionFailed,
                        "state preconditions require an action subject",
                    )
                })?;
            let values = value_fields(&request.input)?;
            let key = primary_key(subject, &values)?;
            let backend = object_backend(&snapshot, subject)?;
            context.charge_backend_call()?;
            let record = backend.get(subject.api_name(), &key)?.ok_or_else(|| {
                Error::new(
                    ErrorCode::PreconditionFailed,
                    "action subject does not exist",
                )
            })?;
            if let Some(expected) = &request.expected_revision
                && record.revision.as_ref() != Some(expected)
            {
                return Err(Error::stale_revision("action subject revision changed"));
            }
            for filter in action
                .preconditions()
                .iter()
                .chain(&request.preconditions)
                .chain(decision.row_filter.iter())
            {
                if !compile_filter(subject, filter)?.matches(&record.values) {
                    return Err(Error::new(
                        ErrorCode::PreconditionFailed,
                        "action state precondition was not satisfied",
                    ));
                }
            }
        }
        let approval_required = decision.require_approval
            || matches!(action.approval(), tesela_ir::ApprovalPolicy::Always)
            || (matches!(action.approval(), tesela_ir::ApprovalPolicy::Policy)
                && !action.is_read_only());
        if approval_required && !context.is_approved(&request.action) {
            return Err(Error::new(
                ErrorCode::ApprovalRequired,
                "action requires approval",
            ));
        }
        let executor = snapshot
            .actions
            .get(action.id().index())
            .and_then(Option::as_ref)
            .ok_or_else(|| {
                Error::unsupported(format!("action '{}' is not bound", request.action))
            })?;
        context.charge_backend_call()?;
        let output = executor.execute(context, &request)?;
        Ok(self.finish(
            context,
            &snapshot,
            Operation::Execute,
            &request.action,
            "action_executor",
            decision,
            action.effects(),
            approval_required,
            output,
            started,
        ))
    }
}
