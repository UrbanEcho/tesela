use super::*;

impl Runtime {
    /// Aggregate through the shared one-pass residual executor.
    pub fn aggregate(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        query: AggregateQuery,
    ) -> Result<Execution<AggregateResult>, Error> {
        let started = Instant::now();
        let snapshot = self.snapshot.load_full();
        let object_type = object_type(&snapshot, object)?;
        let decision = self.authorize(
            context,
            Operation::Aggregate,
            ResourceKind::ObjectType,
            object,
            None,
        )?;
        ensure_approval(context, object, &decision)?;
        let approval_required = decision.require_approval;
        for field in &query.group_by {
            if object_type.property(field).is_none() {
                return Err(Error::unknown_property(object_type, field));
            }
        }
        let filter = match (query.filter.clone(), decision.row_filter.clone()) {
            (Some(a), Some(b)) => Some(Filter::and(vec![a, b])),
            (a, b) => a.or(b),
        };
        if let Some(filter) = &filter {
            compile_filter(object_type, filter)?;
        }
        let backend = object_backend(&snapshot, object_type)?;
        context.charge_backend_call()?;
        let mut effective = query.clone();
        effective.filter = filter.clone();
        effective
            .group_by
            .retain(|field| !decision.redactions.contains(field));
        let guarded = self.guards(context, object);
        let result = if backend.capabilities().aggregate && !guarded {
            backend.aggregate(context, object, &effective)?
        } else {
            let mut accumulator =
                tesela_backend::AggregateAccumulator::new(&query, &decision.redactions);
            // Primary-key order (`sort: []`) keeps the walk stable; the limit is
            // only the page size, and the walk follows cursors until none is left.
            let limit = page_limit(None, self.max_query_limit)?;
            let mut cursor = None;
            let mut walk = CursorWalk::default();
            loop {
                let page = backend.search(
                    context,
                    object,
                    &Query {
                        filter: filter.clone(),
                        sort: Vec::new(),
                        limit: Some(limit),
                        cursor: cursor.clone(),
                        ..Query::default()
                    },
                )?;
                walk.advance(page.records.is_empty(), page.next_cursor.as_ref())?;
                let records = if guarded {
                    self.admit(context, object, page.records)?
                } else {
                    page.records
                };
                context.charge_rows(records.len() as u64)?;
                charge_bytes_with(context, || records_bytes(&records))?;
                accumulator.extend(&records);
                cursor = page.next_cursor;
                if cursor.is_none() {
                    break;
                }
                context.charge_backend_call()?;
            }
            accumulator.finish()
        };
        Ok(self.finish(
            context,
            &snapshot,
            Operation::Aggregate,
            object,
            backend.backend_type(),
            decision,
            tesela_ir::ActionEffects::read_only(),
            approval_required,
            result,
            started,
        ))
    }
}
