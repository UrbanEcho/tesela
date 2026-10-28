//! In-memory backend for tests, examples and local development.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use tesela_core::{ApiName, Error, ExecutionContext, Key, Value, lock_read, lock_write};
use tesela_ir::{AggregateResult, MutationResult, Page, Record, Revision};

use crate::aggregate::Aggregator;
use crate::residual::{Predicate, execute_rows};
use crate::{AggregateQuery, Backend, BackendCapabilities, LinkResolver, Mutation, Query};

mod batch;
mod table;
use batch::{apply, check_revision, roll_back};
use table::{Columns, Table};

type Shared = Arc<RwLock<Table>>;

/// A thread-safe in-memory backend.
///
/// It stores rows and answers queries, and it knows nothing about the ontology:
/// the primary key arrives already extracted, and the only physical fact it
/// holds — which property carries a link's source key — is something the host
/// states explicitly with [`MemoryBackend::bind_link`].
///
/// Each object type owns its table and lock, so work on one type never waits
/// for another. Rows are dense vectors over an interned field dictionary.
pub struct MemoryBackend {
    tables: RwLock<HashMap<ApiName, Shared>>,
    links: RwLock<HashMap<ApiName, ApiName>>,
    revision: AtomicU64,
}

impl MemoryBackend {
    /// Create an empty backend.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tables: RwLock::new(HashMap::new()),
            links: RwLock::new(HashMap::new()),
            revision: AtomicU64::new(1),
        })
    }

    /// Declare that `link` is stored as `target_property` on the target object
    /// type, holding the source record's primary key.
    ///
    /// This is the one piece of physical mapping the in-memory backend needs,
    /// and the host supplies it because only the host knows how its data is
    /// shaped.
    pub fn bind_link(&self, link: ApiName, target_property: ApiName) -> &Self {
        // Poisoning is not possible while nothing panics under the write lock.
        if let Ok(mut links) = self.links.write() {
            links.insert(link, target_property);
        }
        self
    }

    /// How many records an object type holds.
    pub fn len(&self, object_type: &ApiName) -> Result<usize, Error> {
        self.table(object_type)?
            .map_or(Ok(0), |table| Ok(lock_read(&table)?.rows.len()))
    }

    /// Whether an object type holds no records.
    pub fn is_empty(&self, object_type: &ApiName) -> Result<bool, Error> {
        Ok(self.len(object_type)? == 0)
    }

    fn next_revision(&self) -> Revision {
        Revision::new(self.revision.fetch_add(1, Ordering::Relaxed).to_string())
    }

    fn table(&self, object_type: &ApiName) -> Result<Option<Shared>, Error> {
        Ok(lock_read(&self.tables)?.get(object_type).cloned())
    }

    fn table_or_create(&self, object_type: &ApiName) -> Result<Shared, Error> {
        if let Some(table) = self.table(object_type)? {
            return Ok(table);
        }
        Ok(lock_write(&self.tables)?
            .entry(object_type.clone())
            .or_default()
            .clone())
    }
}

impl Backend for MemoryBackend {
    fn backend_type(&self) -> &str {
        "memory"
    }

    fn capabilities(&self) -> BackendCapabilities {
        BackendCapabilities {
            search: true,
            get: true,
            create: true,
            update: true,
            delete: true,
            aggregate: true,
            upsert: true,
            batch: true,
            conditional_mutation: true,
        }
    }

    fn search(
        &self,
        _context: &ExecutionContext,
        object_type: &ApiName,
        query: &Query,
    ) -> Result<Page, Error> {
        let Some(table) = self.table(object_type)? else {
            return Ok(Page::default());
        };
        let table = lock_read(&table)?;
        let page = execute_rows(&Columns(&table), table.rows.values().map(Ok), query)?;
        Ok(Page {
            records: page.rows.into_iter().map(|row| table.record(row)).collect(),
            next_cursor: page.next_cursor,
        })
    }

    fn get(
        &self,
        _context: &ExecutionContext,
        object_type: &ApiName,
        primary_key: &Key,
    ) -> Result<Option<Record>, Error> {
        let Some(table) = self.table(object_type)? else {
            return Ok(None);
        };
        let table = lock_read(&table)?;
        Ok(table.rows.get(primary_key).map(|row| table.record(row)))
    }

    fn create(
        &self,
        _context: &ExecutionContext,
        object_type: &ApiName,
        primary_key: Key,
        values: BTreeMap<ApiName, Value>,
    ) -> Result<MutationResult, Error> {
        let shared = self.table_or_create(object_type)?;
        let mut table = lock_write(&shared)?;
        if table.rows.contains_key(&primary_key) {
            return Err(Error::conflict(format!(
                "record '{primary_key}' already exists in '{object_type}'"
            )));
        }
        let row = table.row(primary_key.clone(), &values, self.next_revision());
        let record = table.record(&row);
        table.replace(&primary_key, Some(row));
        Ok(MutationResult {
            record: Some(record),
            rows_affected: Some(1),
        })
    }

    fn update(
        &self,
        _context: &ExecutionContext,
        object_type: &ApiName,
        primary_key: &Key,
        values: BTreeMap<ApiName, Value>,
        expected_revision: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let shared = self.table_or_create(object_type)?;
        let mut table = lock_write(&shared)?;
        let row = table
            .rows
            .get(primary_key)
            .ok_or_else(|| Error::not_found("record", primary_key))?;
        check_revision(row, expected_revision)?;
        table.patch(primary_key, &values, self.next_revision(), false);
        let record = table.rows.get(primary_key).map(|row| table.record(row));
        Ok(MutationResult {
            record,
            rows_affected: Some(1),
        })
    }

    fn delete(
        &self,
        _context: &ExecutionContext,
        object_type: &ApiName,
        primary_key: &Key,
        expected_revision: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let shared = self.table_or_create(object_type)?;
        let mut table = lock_write(&shared)?;
        if let Some(row) = table.rows.get(primary_key) {
            check_revision(row, expected_revision)?;
        }
        let removed = table.replace(primary_key, None).is_some();
        Ok(MutationResult {
            record: None,
            rows_affected: Some(i64::from(removed)),
        })
    }

    fn upsert(
        &self,
        _context: &ExecutionContext,
        object_type: &ApiName,
        primary_key: Key,
        values: BTreeMap<ApiName, Value>,
        expected_revision: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let shared = self.table_or_create(object_type)?;
        let mut table = lock_write(&shared)?;
        if let Some(existing) = table.rows.get(&primary_key) {
            check_revision(existing, expected_revision)?;
        } else if expected_revision.is_some() {
            return Err(Error::stale_revision(
                "record does not exist at the expected revision",
            ));
        }
        let row = table.row(primary_key.clone(), &values, self.next_revision());
        let record = table.record(&row);
        table.replace(&primary_key, Some(row));
        Ok(MutationResult {
            record: Some(record),
            rows_affected: Some(1),
        })
    }

    fn batch(
        &self,
        _context: &ExecutionContext,
        object_type: &ApiName,
        primary_key: &ApiName,
        mutations: &[Mutation],
    ) -> Result<MutationResult, Error> {
        let shared = self.table_or_create(object_type)?;
        let mut table = lock_write(&shared)?;
        let mut undo = Vec::with_capacity(mutations.len());
        let mut affected = 0i64;
        for mutation in mutations {
            match apply(
                &mut table,
                primary_key,
                mutation,
                || self.next_revision(),
                &mut undo,
            ) {
                Ok(count) => affected += count,
                Err(error) => {
                    roll_back(&mut table, undo);
                    return Err(error);
                }
            }
        }
        Ok(MutationResult {
            record: None,
            rows_affected: Some(affected),
        })
    }

    fn aggregate(
        &self,
        _context: &ExecutionContext,
        object_type: &ApiName,
        query: &AggregateQuery,
    ) -> Result<AggregateResult, Error> {
        let shared = self.table(object_type)?;
        let guard = shared.as_ref().map(|table| lock_read(table)).transpose()?;
        let empty = Table::default();
        let table = guard.as_deref().unwrap_or(&empty);
        let access = Columns(table);
        let filter = query
            .filter
            .as_ref()
            .map(|filter| Predicate::compile(&access, filter))
            .transpose()?;
        let mut aggregator = Aggregator::new(&access, query.group_by.clone(), &query.aggregations);
        for row in table.rows.values() {
            if filter
                .as_ref()
                .is_none_or(|filter| filter.matches(&access, &row))
            {
                aggregator.add(&access, &row);
            }
        }
        Ok(aggregator.finish())
    }
}

impl LinkResolver for MemoryBackend {
    fn linked_keys(
        &self,
        _context: &ExecutionContext,
        link: &ApiName,
        target: &ApiName,
        source_key: &Key,
    ) -> Result<Vec<Key>, Error> {
        let property = lock_read(&self.links)?.get(link).cloned().ok_or_else(|| {
            Error::unsupported(format!("link '{link}' is not bound in this memory backend"))
        })?;
        let Some(table) = self.table(target)? else {
            return Ok(Vec::new());
        };
        if let Some(keys) = lock_read(&table)?.lookup(&property, source_key) {
            return Ok(keys);
        }
        Ok(lock_write(&table)?.linked(&property, source_key))
    }
}

#[cfg(test)]
mod tests;
