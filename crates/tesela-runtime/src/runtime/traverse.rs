//! Traversal across independently bound source, link and target backends.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use super::*;

/// Linked keys looked up per target query, and the page size of each of that
/// query's own cursor walk. A bound on one backend call, never on rows.
const LINK_CHUNK: usize = 256;

/// A record ranked by the query's total order.
struct Ranked<'a> {
    record: Record,
    sort: &'a [Sort],
}

impl PartialEq for Ranked<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
impl Eq for Ranked<'_> {}
impl PartialOrd for Ranked<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ranked<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        compare_records(&self.record, &other.record, self.sort)
    }
}

/// Keeps the `keep` first records of a stream in the query's total order, so a
/// merge of unordered sub-walks holds O(keep) rows however many it sees.
struct KeysetTopK<'a> {
    keep: usize,
    sort: &'a [Sort],
    heap: BinaryHeap<Ranked<'a>>,
}

impl<'a> KeysetTopK<'a> {
    fn new(keep: usize, sort: &'a [Sort]) -> Self {
        Self {
            keep,
            sort,
            heap: BinaryHeap::new(),
        }
    }

    fn offer(&mut self, record: Record) {
        let entry = Ranked {
            record,
            sort: self.sort,
        };
        if self.heap.len() < self.keep {
            self.heap.push(entry);
        } else if self.heap.peek().is_some_and(|worst| entry < *worst) {
            self.heap.pop();
            self.heap.push(entry);
        }
    }

    fn into_sorted(self) -> Vec<Record> {
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|entry| entry.record)
            .collect()
    }
}

impl Runtime {
    /// Traverse across independently bound source, link, and target backends.
    ///
    /// The resolver returns the complete adjacency key list
    /// ([`LinkResolver::linked_keys`]); that is materialization, not
    /// truncation, and can become a paged resolver later. Target rows are read
    /// in chunks of linked keys, each chunk followed through its own cursors to
    /// exhaustion, and merged into the page by the shared total order. The
    /// returned cursor is scoped to the source record, link, target, filter and
    /// sort, so the walk reaches every linked row exactly once.
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
        let source_page = source_backend.search(
            context,
            source.api_name(),
            &Query {
                filter: Some(source_filter),
                limit: Some(1),
                ..Query::default()
            },
        )?;
        let Some(source_record) = source_page.records.into_iter().next() else {
            return Err(Error::not_found("record", &query.source_pk));
        };
        if self.guards(context, source.api_name())
            && !self
                .policy
                .admit_record(context, source.api_name(), &source_record)?
        {
            return Err(Error::policy_denied(
                "source record is outside the permitted access scope",
            ));
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
        check_fields(target, query.fields.as_deref())?;
        decision
            .redactions
            .extend(target_decision.redactions.iter().cloned());
        check_sort_readable(&query.sort, &decision.redactions)?;
        if let Some(cursor) = &query.cursor {
            cursor.check_token()?;
        }
        let limit = effective_limit(query.limit.map(i64::from), self.max_query_limit)?;
        let scope = format!(
            "traverse:{}:{}:{link_name}:{}",
            source.api_name(),
            query.source_pk,
            target.api_name()
        );
        let position = query
            .cursor
            .as_ref()
            .map(|cursor| cursor.decode(&scope, base_filter.as_ref(), &query.sort))
            .transpose()?;
        let after = position
            .as_ref()
            .map(|position| position.to_filter(&query.sort, target.primary_key().api_name()));
        // Bounded state: at most `limit + 1` rows survive, plus one sub-page.
        let guarded = self.guards(context, target.api_name());
        let mut best = KeysetTopK::new(limit.saturating_add(1), &query.sort);
        for chunk in keys.chunks(LINK_CHUNK) {
            let mut conjunction = vec![Filter::is_in(
                target.primary_key().api_name().clone(),
                chunk.iter().map(Key::to_value).collect(),
            )];
            conjunction.extend(base_filter.clone());
            conjunction.extend(after.clone());
            let sub_query = Query {
                filter: Some(Filter::and(conjunction)),
                sort: query.sort.clone(),
                limit: Some(LINK_CHUNK as i32),
                cursor: None,
                fields: None,
            };
            // Follow this chunk's own cursors to exhaustion: the final page of
            // the traversal must see every linked row that follows the position.
            let mut sub_cursor = None;
            let mut walk = CursorWalk::default();
            loop {
                context.check(Operation::Traverse, link_name)?;
                context.charge_backend_call()?;
                let page = target_backend.search(
                    context,
                    target.api_name(),
                    &Query {
                        cursor: sub_cursor.take(),
                        ..sub_query.clone()
                    },
                )?;
                walk.advance(page.records.is_empty(), page.next_cursor.as_ref())?;
                let records = if guarded {
                    self.admit(context, target.api_name(), page.records)?
                } else {
                    page.records
                };
                for record in records {
                    if position
                        .as_ref()
                        .is_none_or(|position| position.is_after(&record, &query.sort))
                    {
                        best.offer(record);
                    }
                }
                sub_cursor = page.next_cursor;
                if sub_cursor.is_none() {
                    break;
                }
            }
        }
        let mut records = best.into_sorted();
        let has_more = records.len() > limit;
        records.truncate(limit);
        // Charge the rows returned, as `search` does; scanned rows are not
        // charged, so following cursors never exhausts the budget early.
        context.charge_rows(records.len() as u64)?;
        let next_cursor = records
            .last()
            .filter(|_| has_more)
            .map(|last| Cursor::encode(&scope, base_filter.as_ref(), &query.sort, last));
        let mut page = Page {
            records,
            next_cursor,
        };
        decision.redactions.sort();
        decision.redactions.dedup();
        decision.obligations.extend(target_decision.obligations);
        redact(&mut page.records, &decision.redactions);
        charge_bytes_with(context, || records_bytes(&page.records))?;
        project_fields(
            &mut page.records,
            query.fields.as_deref(),
            target.primary_key().api_name(),
        );
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
