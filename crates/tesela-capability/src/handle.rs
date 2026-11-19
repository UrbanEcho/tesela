//! Bounded, execution-scoped result handles kept outside model context.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use serde::{Serialize, Serializer};
use tesela_core::{ApiName, Error, ExecutionContext, Key};
use tesela_ir::order::{Sort, compare_records};
use tesela_ir::{Cursor, Record};

/// Bounded execution-scoped records retained outside model context.
#[derive(Debug, Clone)]
pub struct ResultHandle {
    id: Arc<str>,
    request_id: Arc<str>,
    records: Arc<[Record]>,
    sort: Arc<[Sort]>,
    partial: bool,
}

/// Serializes as a summary (`id` and `count`) so records never enter model context.
impl Serialize for ResultHandle {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("ResultHandle", 2)?;
        state.serialize_field("id", self.id())?;
        state.serialize_field("count", &self.records.len())?;
        state.end()
    }
}

impl ResultHandle {
    /// Retain `records`, ordered by `sort` and then by primary key.
    ///
    /// Records arrive in that order from a search, so sorting is a cheap
    /// verification pass. The order is the handle's own: it is defined over the
    /// retained values, so a read walks the same total order the cursor names.
    /// The rows are charged to the execution's row budget.
    pub fn new(
        context: &ExecutionContext,
        id: impl AsRef<str>,
        mut records: Vec<Record>,
        sort: impl Into<Arc<[Sort]>>,
    ) -> Result<Self, Error> {
        let sort = sort.into();
        if !records
            .windows(2)
            .all(|pair| compare_records(&pair[0], &pair[1], &sort) != Ordering::Greater)
        {
            records.sort_by(|left, right| compare_records(left, right, &sort));
        }
        context.charge_rows(records.len() as u64)?;
        Ok(Self {
            id: Arc::from(id.as_ref()),
            request_id: Arc::from(context.request_id()),
            records: records.into(),
            sort,
            partial: false,
        })
    }
    /// Mark the handle as one page of a larger query, whose `next_cursor` was
    /// not exhausted. Consumers that need every row refuse a partial handle
    /// instead of expanding a truncated key list.
    #[must_use]
    pub fn with_partial(mut self, partial: bool) -> Self {
        self.partial = partial;
        self
    }
    /// Whether the retained records are only a page of a larger result.
    #[must_use]
    pub fn is_partial(&self) -> bool {
        self.partial
    }
    /// Handle identifier.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Number of retained records.
    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }
    /// Whether the handle retains no records.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    /// Page retained records strictly after `after`, at most `limit` of them.
    ///
    /// The cursor is scoped to this handle and its sort, so a cursor from
    /// another handle or query is rejected. The returned cursor is `None`
    /// exactly when no retained record follows the page.
    pub fn page(
        &self,
        context: &ExecutionContext,
        after: Option<&Cursor>,
        limit: NonZeroUsize,
    ) -> Result<(Vec<Record>, Option<Cursor>), Error> {
        self.check_context(context)?;
        let start = match after {
            Some(cursor) => {
                let position = cursor.decode(&self.id, None, &self.sort)?;
                self.records
                    .partition_point(|record| !position.is_after(record, &self.sort))
            }
            None => 0,
        };
        let rest = &self.records[start..];
        let page = rest[..rest.len().min(limit.get())].to_vec();
        let next = (page.len() < rest.len())
            .then(|| page.last())
            .flatten()
            .map(|last| Cursor::encode(&self.id, None, &self.sort, last));
        Ok((page, next))
    }
    /// Iterate every retained record in handle order, for internal consumers
    /// that must see all rows.
    pub fn iter(&self, context: &ExecutionContext) -> Result<impl Iterator<Item = &Record>, Error> {
        self.check_context(context)?;
        Ok(self.records.iter())
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
