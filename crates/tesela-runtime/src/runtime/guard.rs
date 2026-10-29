//! Per-record read guards for policies a row filter cannot express.

use super::*;

/// Extra backend pages fetched to fill a page that the guard thinned out.
const MAX_REFILLS: usize = 8;

impl Runtime {
    /// Whether the policy wants every returned record of `object` checked.
    pub(super) fn guards(&self, context: &ExecutionContext, object: &ApiName) -> bool {
        self.policy.guards_records(context, object)
    }

    /// Keep only the records the policy admits.
    pub(super) fn admit(
        &self,
        context: &ExecutionContext,
        object: &ApiName,
        records: Vec<Record>,
    ) -> Result<Vec<Record>, Error> {
        let mut admitted = Vec::with_capacity(records.len());
        for record in records {
            if self.policy.admit_record(context, object, &record)? {
                admitted.push(record);
            }
        }
        Ok(admitted)
    }

    /// Search a backend, applying the record guard and refilling a thinned page.
    ///
    /// A refill asks only for the rows still missing, so the page never exceeds
    /// the requested limit and the returned cursor stays exact.
    pub(super) fn search_guarded(
        &self,
        context: &ExecutionContext,
        backend: &dyn Backend,
        object: &ApiName,
        query: &Query,
    ) -> Result<Page, Error> {
        let mut page = backend.search(context, object, query)?;
        if !self.guards(context, object) {
            return Ok(page);
        }
        let wanted = query
            .limit
            .map_or(usize::MAX, |limit| limit.max(0) as usize);
        let mut records = self.admit(context, object, std::mem::take(&mut page.records))?;
        let mut refills = 0;
        while records.len() < wanted && page.next_cursor.is_some() && refills < MAX_REFILLS {
            refills += 1;
            context.charge_backend_call()?;
            let missing = wanted - records.len();
            let next = Query {
                limit: Some(i32::try_from(missing).unwrap_or(i32::MAX)),
                offset: None,
                cursor: page.next_cursor.take(),
                ..query.clone()
            };
            page = backend.search(context, object, &next)?;
            records.extend(self.admit(context, object, std::mem::take(&mut page.records))?);
        }
        page.records = records;
        Ok(page)
    }
}
