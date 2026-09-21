//! In-memory backend for tests, examples and local development.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use tesela_core::{ApiName, Error, Key, Value, lock_read, lock_write};
use tesela_ir::{AggregateResult, MutationResult, Page, Record, Revision};

use crate::{
    AggregateQuery, Backend, BackendCapabilities, LinkResolver, Mutation, Query, execute_residual,
};

mod query;
use query::*;

/// A thread-safe in-memory backend.
///
/// It stores rows and answers queries, and it knows nothing about the ontology:
/// the primary key arrives already extracted, and the only physical fact it
/// holds — which property carries a link's source key — is something the host
/// states explicitly with [`MemoryBackend::bind_link`].
pub struct MemoryBackend {
    records: RwLock<HashMap<ApiName, BTreeMap<Key, Record>>>,
    links: RwLock<HashMap<ApiName, ApiName>>,
    revision: AtomicU64,
}

impl MemoryBackend {
    /// Create an empty backend.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            records: RwLock::new(HashMap::new()),
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
        Ok(lock_read(&self.records)?
            .get(object_type)
            .map_or(0, BTreeMap::len))
    }

    /// Whether an object type holds no records.
    pub fn is_empty(&self, object_type: &ApiName) -> Result<bool, Error> {
        Ok(self.len(object_type)? == 0)
    }

    fn next_revision(&self) -> Revision {
        Revision::new(self.revision.fetch_add(1, Ordering::Relaxed).to_string())
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

    fn search(&self, object_type: &ApiName, query: &Query) -> Result<Page, Error> {
        let records = lock_read(&self.records)?;
        let Some(records_for_type) = records.get(object_type) else {
            return Ok(Page::default());
        };
        execute_residual(records_for_type.values(), query)
    }

    fn get(&self, object_type: &ApiName, primary_key: &Key) -> Result<Option<Record>, Error> {
        Ok(lock_read(&self.records)?
            .get(object_type)
            .and_then(|records_for_type| records_for_type.get(primary_key))
            .cloned())
    }

    fn create(
        &self,
        object_type: &ApiName,
        primary_key: Key,
        values: BTreeMap<ApiName, Value>,
    ) -> Result<MutationResult, Error> {
        let mut records = lock_write(&self.records)?;
        let records_for_type = records.entry(object_type.clone()).or_default();
        if records_for_type.contains_key(&primary_key) {
            return Err(Error::conflict(format!(
                "record '{primary_key}' already exists in '{object_type}'"
            )));
        }
        let mut record = Record::new(primary_key.clone(), values);
        record.revision = Some(self.next_revision());
        records_for_type.insert(primary_key, record.clone());
        Ok(MutationResult {
            record: Some(record),
            rows_affected: Some(1),
        })
    }

    fn update(
        &self,
        object_type: &ApiName,
        primary_key: &Key,
        values: BTreeMap<ApiName, Value>,
        expected_revision: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let mut records = lock_write(&self.records)?;
        let records_for_type = records.entry(object_type.clone()).or_default();
        let record = records_for_type
            .get_mut(primary_key)
            .ok_or_else(|| Error::not_found("record", primary_key))?;
        check_revision(record, expected_revision)?;
        for (field, value) in values {
            record.values.insert(field, value);
        }
        record.revision = Some(self.next_revision());
        Ok(MutationResult {
            record: Some(record.clone()),
            rows_affected: Some(1),
        })
    }

    fn delete(
        &self,
        object_type: &ApiName,
        primary_key: &Key,
        expected_revision: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let mut records = lock_write(&self.records)?;
        let records_for_type = records.entry(object_type.clone()).or_default();
        if let Some(record) = records_for_type.get(primary_key) {
            check_revision(record, expected_revision)?;
        }
        let removed = records_for_type.remove(primary_key).is_some();
        Ok(MutationResult {
            record: None,
            rows_affected: Some(i64::from(removed)),
        })
    }

    fn upsert(
        &self,
        object_type: &ApiName,
        primary_key: Key,
        values: BTreeMap<ApiName, Value>,
        expected_revision: Option<&Revision>,
    ) -> Result<MutationResult, Error> {
        let mut records = lock_write(&self.records)?;
        let records_for_type = records.entry(object_type.clone()).or_default();
        if let Some(existing) = records_for_type.get(&primary_key) {
            check_revision(existing, expected_revision)?;
        } else if expected_revision.is_some() {
            return Err(Error::stale_revision(
                "record does not exist at the expected revision",
            ));
        }
        let mut record = Record::new(primary_key.clone(), values);
        record.revision = Some(self.next_revision());
        records_for_type.insert(primary_key, record.clone());
        Ok(MutationResult {
            record: Some(record),
            rows_affected: Some(1),
        })
    }

    fn batch(
        &self,
        object_type: &ApiName,
        primary_key: &ApiName,
        mutations: &[Mutation],
    ) -> Result<MutationResult, Error> {
        let mut records = lock_write(&self.records)?;
        let current = records.entry(object_type.clone()).or_default();
        let mut staged = current.clone();
        let mut affected = 0i64;
        for mutation in mutations {
            affected += apply_staged(&mut staged, primary_key, mutation, || self.next_revision())?;
        }
        *current = staged;
        Ok(MutationResult {
            record: None,
            rows_affected: Some(affected),
        })
    }

    fn aggregate(
        &self,
        object_type: &ApiName,
        query: &AggregateQuery,
    ) -> Result<AggregateResult, Error> {
        let page = self.search(
            object_type,
            &Query {
                filter: query.filter.clone(),
                limit: Some(i32::MAX),
                ..Query::default()
            },
        )?;
        Ok(compute_aggregate(
            &page.records,
            &query.group_by,
            &query.aggregations,
        ))
    }
}

impl LinkResolver for MemoryBackend {
    fn linked_keys(
        &self,
        link: &ApiName,
        target: &ApiName,
        source_key: &Key,
    ) -> Result<Vec<Key>, Error> {
        let property = lock_read(&self.links)?.get(link).cloned().ok_or_else(|| {
            Error::unsupported(format!("link '{link}' is not bound in this memory backend"))
        })?;
        let records = lock_read(&self.records)?;
        let Some(records_for_type) = records.get(target) else {
            return Ok(Vec::new());
        };
        Ok(records_for_type
            .values()
            .filter(|record| {
                record
                    .values
                    .get(&property)
                    .map(Key::from)
                    .is_some_and(|value| value == *source_key)
            })
            .map(|record| record.primary_key.clone())
            .collect())
    }
}
