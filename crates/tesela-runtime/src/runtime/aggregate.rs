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
        let result = if backend.capabilities().aggregate {
            backend.aggregate(context, object, &effective)?
        } else {
            let mut accumulator =
                tesela_backend::AggregateAccumulator::new(&query, &decision.redactions);
            let mut cursor = None;
            loop {
                let page = backend.search(
                    context,
                    object,
                    &Query {
                        filter: filter.clone(),
                        limit: Some(self.max_query_limit as i32),
                        cursor: cursor.clone(),
                        ..Query::default()
                    },
                )?;
                context.charge_rows(page.records.len() as u64)?;
                charge_bytes_with(context, || records_bytes(&page.records))?;
                accumulator.extend(&page.records);
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

    /// Traverse across independently bound source, link, and target backends.
    pub fn traverse(
        &self,
        context: &ExecutionContext,
        link_name: &ApiName,
        query: TraversalQuery,
    ) -> Result<Execution<Page>, Error> {
        let started = Instant::now();
        let snapshot = self.snapshot.load_full();
        context.check(Operation::Traverse, link_name)?;
        let link = snapshot.ontology.link_type(link_name).ok_or_else(|| {
            Error::new(
                ErrorCode::UnknownLink,
                format!("unknown link '{link_name}'"),
            )
        })?;
        let source = snapshot
            .ontology
            .object_type_by_id(link.from())
            .ok_or_else(|| Error::internal("link source missing"))?;
        let target = snapshot
            .ontology
            .object_type_by_id(link.to())
            .ok_or_else(|| Error::internal("link target missing"))?;
        let source_decision = self.authorize(
            context,
            Operation::Read,
            ResourceKind::ObjectType,
            source.api_name(),
            None,
        )?;
        ensure_approval(context, source.api_name(), &source_decision)?;
        let source_backend = object_backend(&snapshot, source)?;
        context.charge_backend_call()?;
        let source_filter = Filter::eq(
            source.primary_key().api_name().clone(),
            query.source_pk.to_value(),
        );
        let source_filter = source_decision
            .row_filter
            .map_or(source_filter.clone(), |policy| {
                Filter::and(vec![source_filter, policy])
            });
        if source_backend
            .search(
                context,
                source.api_name(),
                &Query {
                    filter: Some(source_filter),
                    limit: Some(1),
                    ..Query::default()
                },
            )?
            .records
            .is_empty()
        {
            return Err(Error::not_found("record", &query.source_pk));
        }
        let mut decision = self.authorize(
            context,
            Operation::Traverse,
            ResourceKind::LinkType,
            link_name,
            None,
        )?;
        ensure_approval(context, link_name, &decision)?;
        let target_decision = self.authorize(
            context,
            Operation::Read,
            ResourceKind::ObjectType,
            target.api_name(),
            None,
        )?;
        ensure_approval(context, target.api_name(), &target_decision)?;
        let approval_required = decision.require_approval
            || source_decision.require_approval
            || target_decision.require_approval;
        let resolver = snapshot
            .links
            .get(link.id().index())
            .and_then(Option::as_ref)
            .ok_or_else(|| Error::unsupported(format!("link '{link_name}' is not bound")))?;
        context.charge_backend_call()?;
        let keys = resolver.linked_keys(context, link_name, target.api_name(), &query.source_pk)?;
        let target_backend = object_backend(&snapshot, target)?;
        let mut base_filter = query.filter;
        for policy in [
            decision.row_filter.clone(),
            target_decision.row_filter.clone(),
        ]
        .into_iter()
        .flatten()
        {
            base_filter = Some(base_filter.map_or(policy.clone(), |existing| {
                Filter::and(vec![existing, policy])
            }));
        }
        let limit = effective_limit(query.limit, self.max_query_limit);
        let offset = query.offset.unwrap_or_default().max(0) as usize;
        let keep = offset.saturating_add(limit).saturating_add(1);
        let mut records = Vec::with_capacity(keep.min(keys.len()));
        for chunk in keys.chunks(256) {
            context.check(Operation::Traverse, link_name)?;
            let key_filter = Filter::is_in(
                target.primary_key().api_name().clone(),
                chunk.iter().map(Key::to_value).collect(),
            );
            let filter = Some(base_filter.clone().map_or(key_filter.clone(), |existing| {
                Filter::and(vec![existing, key_filter])
            }));
            context.charge_backend_call()?;
            let page = target_backend.search(
                context,
                target.api_name(),
                &Query {
                    filter,
                    sort: query.sort.clone(),
                    limit: Some(keep as i32),
                    offset: None,
                    cursor: None,
                },
            )?;
            records.extend(page.records);
            records.sort_unstable_by(|left, right| compare_records(left, right, &query.sort));
            records.truncate(keep);
        }
        let has_more = records.len() > offset.saturating_add(limit);
        let records = records.into_iter().skip(offset).take(limit).collect();
        let mut page = Page {
            records,
            next_cursor: has_more.then(|| (offset + limit).to_string()),
        };
        decision.redactions.extend(target_decision.redactions);
        decision.redactions.sort();
        decision.redactions.dedup();
        decision.obligations.extend(target_decision.obligations);
        redact(&mut page.records, &decision.redactions);
        context.charge_rows(page.records.len() as u64)?;
        charge_bytes_with(context, || records_bytes(&page.records))?;
        Ok(self.finish(
            context,
            &snapshot,
            Operation::Traverse,
            link_name,
            target_backend.backend_type(),
            decision,
            tesela_ir::ActionEffects::read_only(),
            approval_required,
            page,
            started,
        ))
    }
}
