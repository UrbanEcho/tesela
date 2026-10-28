//! Bounded, execution-scoped result handles kept outside model context.

use std::collections::BTreeMap;
use std::sync::Arc;

use tesela_core::{ApiName, Error, ExecutionContext, Key};
use tesela_ir::Record;

/// Bounded execution-scoped records retained outside model context.
#[derive(Debug, Clone)]
pub struct ResultHandle {
    id: Arc<str>,
    request_id: Arc<str>,
    records: Arc<[Record]>,
}

impl ResultHandle {
    /// Create a bounded handle.
    pub fn new(
        context: &ExecutionContext,
        id: impl AsRef<str>,
        records: Vec<Record>,
        max_rows: usize,
    ) -> Result<Self, Error> {
        if records.len() > max_rows {
            return Err(Error::budget_exhausted("result handle row budget exceeded"));
        }
        context.charge_rows(records.len() as u64)?;
        Ok(Self {
            id: Arc::from(id.as_ref()),
            request_id: Arc::from(context.request_id()),
            records: records.into(),
        })
    }
    /// Handle identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Page retained records.
    pub fn page(
        &self,
        context: &ExecutionContext,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<Record>, Error> {
        self.check_context(context)?;
        Ok(self
            .records
            .iter()
            .skip(offset)
            .take(limit)
            .cloned()
            .collect())
    }
    /// Project selected fields.
    pub fn project(
        &self,
        context: &ExecutionContext,
        fields: &[ApiName],
    ) -> Result<Vec<Record>, Error> {
        self.check_context(context)?;
        Ok(self
            .records
            .iter()
            .cloned()
            .map(|mut record| {
                record.values.retain(|field, _| fields.contains(field));
                record
            })
            .collect())
    }
    /// Filter retained records with a caller-compiled predicate.
    pub fn filter(
        &self,
        context: &ExecutionContext,
        mut predicate: impl FnMut(&Record) -> bool,
    ) -> Result<Vec<Record>, Error> {
        self.check_context(context)?;
        Ok(self
            .records
            .iter()
            .filter(|record| predicate(record))
            .cloned()
            .collect())
    }
    /// Count rows by a field value.
    pub fn count_by(
        &self,
        context: &ExecutionContext,
        field: &ApiName,
    ) -> Result<BTreeMap<Key, u64>, Error> {
        self.check_context(context)?;
        let mut counts = BTreeMap::new();
        for record in self.records.iter() {
            if let Some(value) = record.values.get(field) {
                *counts.entry(Key::from(value)).or_default() += 1;
            }
        }
        Ok(counts)
    }

    fn check_context(&self, context: &ExecutionContext) -> Result<(), Error> {
        if self.request_id.as_ref() == context.request_id() {
            Ok(())
        } else {
            Err(Error::unauthorized(
                "result handle belongs to a different execution context",
            ))
        }
    }
}
