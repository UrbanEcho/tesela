use super::*;

impl Runtime {
    /// Search records.
    pub fn search(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        mut query: Query,
    ) -> Result<Execution<Page>, Error> {
        let started = Instant::now();
        let snapshot = self.snapshot.load_full();
        let object_type = object_type(&snapshot, object)?;
        let decision = self.authorize(
            context,
            Operation::Search,
            ResourceKind::ObjectType,
            object,
            None,
        )?;
        ensure_approval(context, object, &decision)?;
        let approval_required = decision.require_approval;
        if let Some(filter) = decision.row_filter.clone() {
            query = query.and_filter(filter);
        }
        query.limit = Some(effective_limit(query.limit, self.max_query_limit) as i32);
        check_query(object_type, &query)?;
        let backend = object_backend(&snapshot, object_type)?;
        context.charge_backend_call()?;
        let mut page = backend.search(context, object, &query)?;
        redact(&mut page.records, &decision.redactions);
        context.charge_rows(page.records.len() as u64)?;
        context.charge_bytes(records_bytes(&page.records))?;
        Ok(self.finish(
            context,
            &snapshot,
            Operation::Search,
            object,
            backend.backend_type(),
            decision,
            tesela_ir::ActionEffects::read_only(),
            approval_required,
            page,
            started,
        ))
    }

    /// Look up one record while enforcing row policy.
    pub fn lookup(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        key: &Key,
    ) -> Result<Execution<Option<Record>>, Error> {
        let started = Instant::now();
        let snapshot = self.snapshot.load_full();
        let object_type = object_type(&snapshot, object)?;
        let decision = self.authorize(
            context,
            Operation::Read,
            ResourceKind::ObjectType,
            object,
            None,
        )?;
        ensure_approval(context, object, &decision)?;
        let approval_required = decision.require_approval;
        let backend = object_backend(&snapshot, object_type)?;
        context.charge_backend_call()?;
        let mut record = if let Some(policy_filter) = decision.row_filter.clone() {
            let query = Query {
                filter: Some(Filter::and(vec![
                    Filter::eq(object_type.primary_key().api_name().clone(), key.to_value()),
                    policy_filter,
                ])),
                limit: Some(1),
                ..Query::default()
            };
            check_query(object_type, &query)?;
            backend
                .search(context, object, &query)?
                .records
                .into_iter()
                .next()
        } else {
            backend.get(context, object, key)?
        };
        if let Some(record) = record.as_mut() {
            redact(std::slice::from_mut(record), &decision.redactions);
            context.charge_rows(1)?;
            context.charge_bytes(records_bytes(std::slice::from_ref(record)))?;
        }
        Ok(self.finish(
            context,
            &snapshot,
            Operation::Read,
            object,
            backend.backend_type(),
            decision,
            tesela_ir::ActionEffects::read_only(),
            approval_required,
            record,
            started,
        ))
    }

    /// Fetch one record, returning a structured not-found error when absent.
    pub fn get(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        key: &Key,
    ) -> Result<Execution<Record>, Error> {
        let execution = self.lookup(context, object, key)?;
        Ok(Execution {
            output: execution
                .output
                .ok_or_else(|| Error::not_found("record", key))?,
            receipt: execution.receipt,
        })
    }

    /// Execute an atomic backend mutation.
    pub fn mutate(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        mutation: Mutation,
    ) -> Result<Execution<MutationResult>, Error> {
        let started = Instant::now();
        let snapshot = self.snapshot.load_full();
        let object_type = object_type(&snapshot, object)?;
        let policy_input = mutation_input(&mutation);
        let decision = self.authorize(
            context,
            Operation::Mutate,
            ResourceKind::ObjectType,
            object,
            policy_input.as_ref(),
        )?;
        if decision.require_approval && !context.is_approved(object) {
            return Err(Error::new(
                ErrorCode::ApprovalRequired,
                "mutation requires approval",
            ));
        }
        let backend = object_backend(&snapshot, object_type)?;
        let mutation =
            prepare_mutation(context, backend.as_ref(), object_type, mutation, &decision)?;
        if let Some(input) = mutation_input(&mutation) {
            context.charge_bytes(value_bytes(&input))?;
        }
        validate_mutation(&snapshot.ontology, object_type, &mutation)?;
        let capabilities = backend.capabilities();
        if matches!(mutation, Mutation::Upsert { .. }) && !capabilities.upsert {
            return Err(Error::unsupported("backend does not provide atomic upsert"));
        }
        if matches!(mutation, Mutation::Batch { .. }) && !capabilities.batch {
            return Err(Error::unsupported("backend does not provide atomic batch"));
        }
        context.charge_backend_call()?;
        let result = apply_mutation(
            context,
            &snapshot.ontology,
            backend.as_ref(),
            object_type,
            mutation,
        )?;
        let approval_required = decision.require_approval;
        Ok(self.finish(
            context,
            &snapshot,
            Operation::Mutate,
            object,
            backend.backend_type(),
            decision,
            tesela_ir::ActionEffects::new(),
            approval_required,
            result,
            started,
        ))
    }
}

fn mutation_input(mutation: &Mutation) -> Option<Value> {
    let fields = match mutation {
        Mutation::Create { values }
        | Mutation::Update { values, .. }
        | Mutation::Upsert { values, .. } => values,
        Mutation::Delete { .. } | Mutation::Batch { .. } => return None,
    };
    Some(Value::new(serde_json::Value::Object(
        fields
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), value.clone().into()))
            .collect(),
    )))
}

fn prepare_mutation(
    context: &ExecutionContext,
    backend: &dyn Backend,
    object: &CompiledObjectType,
    mutation: Mutation,
    decision: &PolicyDecision,
) -> Result<Mutation, Error> {
    if matches!(mutation, Mutation::Batch { .. })
        && (decision.row_filter.is_some()
            || !decision.input_constraints.is_empty()
            || !decision.transformations.is_empty())
    {
        return Err(Error::unsupported(
            "policy-scoped batch requires native policy-aware batch support",
        ));
    }

    let check_values = |values: &BTreeMap<ApiName, Value>| -> Result<(), Error> {
        for constraint in &decision.input_constraints {
            if !compile_filter(object, constraint)?.matches(values) {
                return Err(Error::new(
                    ErrorCode::PreconditionFailed,
                    "mutation input constraint was not satisfied",
                ));
            }
        }
        Ok(())
    };
    let check_row = |values: &BTreeMap<ApiName, Value>| -> Result<(), Error> {
        if let Some(filter) = &decision.row_filter
            && !compile_filter(object, filter)?.matches(values)
        {
            return Err(Error::policy_denied(
                "record is outside the permitted row scope",
            ));
        }
        Ok(())
    };
    let transform = |values: &mut BTreeMap<ApiName, Value>| {
        values.extend(decision.transformations.clone());
    };
    let load = |key: &Key| -> Result<Record, Error> {
        context.charge_backend_call()?;
        backend
            .get(context, object.api_name(), key)?
            .ok_or_else(|| Error::not_found("record", key))
    };

    match mutation {
        Mutation::Create { mut values } => {
            transform(&mut values);
            check_values(&values)?;
            check_row(&values)?;
            Ok(Mutation::Create { values })
        }
        Mutation::Update {
            primary_key,
            mut values,
            expected_revision,
            preconditions,
        } => {
            transform(&mut values);
            check_values(&values)?;
            let expected_revision = if decision.row_filter.is_some() || !preconditions.is_empty() {
                let current = load(&primary_key)?;
                check_row(&current.values)?;
                check_preconditions(object, &current, &preconditions)?;
                let mut resulting = current.values.clone();
                resulting.extend(values.clone());
                check_row(&resulting)?;
                match expected_revision {
                    Some(expected) if current.revision.as_ref() != Some(&expected) => {
                        return Err(Error::stale_revision("record revision changed"));
                    }
                    Some(expected) => Some(expected),
                    None => current.revision,
                }
            } else {
                expected_revision
            };
            Ok(Mutation::Update {
                primary_key,
                values,
                expected_revision,
                preconditions,
            })
        }
        Mutation::Delete {
            primary_key,
            expected_revision,
            preconditions,
        } => {
            let expected_revision = if decision.row_filter.is_some() || !preconditions.is_empty() {
                let current = load(&primary_key)?;
                check_row(&current.values)?;
                check_preconditions(object, &current, &preconditions)?;
                match expected_revision {
                    Some(expected) if current.revision.as_ref() != Some(&expected) => {
                        return Err(Error::stale_revision("record revision changed"));
                    }
                    Some(expected) => Some(expected),
                    None => current.revision,
                }
            } else {
                expected_revision
            };
            Ok(Mutation::Delete {
                primary_key,
                expected_revision,
                preconditions,
            })
        }
        Mutation::Upsert {
            mut values,
            expected_revision,
            preconditions,
        } => {
            transform(&mut values);
            check_values(&values)?;
            check_row(&values)?;
            let expected_revision = if decision.row_filter.is_some() || !preconditions.is_empty() {
                let key = primary_key(object, &values)?;
                let current = load(&key)?;
                check_row(&current.values)?;
                check_preconditions(object, &current, &preconditions)?;
                match expected_revision {
                    Some(expected) if current.revision.as_ref() != Some(&expected) => {
                        return Err(Error::stale_revision("record revision changed"));
                    }
                    Some(expected) => Some(expected),
                    None => current.revision,
                }
            } else {
                expected_revision
            };
            Ok(Mutation::Upsert {
                values,
                expected_revision,
                preconditions,
            })
        }
        Mutation::Batch { items } => Ok(Mutation::Batch { items }),
    }
}

fn check_preconditions(
    object: &CompiledObjectType,
    record: &Record,
    preconditions: &[Filter],
) -> Result<(), Error> {
    for precondition in preconditions {
        if !compile_filter(object, precondition)?.matches(&record.values) {
            return Err(Error::new(
                ErrorCode::PreconditionFailed,
                "mutation precondition was not satisfied",
            ));
        }
    }
    Ok(())
}
